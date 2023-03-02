use indexmap::IndexMap;
use itertools::Itertools;
use rayon::prelude::*;

use crate::cli::args::runtime::{RuntimeArg, RuntimeArgVersion};
use crate::config::Config;
use crate::env;
use crate::toolset::tool_version::ToolVersionType;
use crate::toolset::{ToolSource, ToolVersion, Toolset};

#[derive(Debug)]
pub struct ToolsetBuilder {
    args: Vec<RuntimeArg>,
    install_missing: bool,
}

impl ToolsetBuilder {
    pub fn new() -> Self {
        Self {
            args: Vec::new(),
            install_missing: false,
        }
    }

    pub fn with_args(mut self, args: &[RuntimeArg]) -> Self {
        self.args = args.to_vec();
        self
    }

    pub fn with_install_missing(mut self) -> Self {
        self.install_missing = true;
        self
    }

    #[tracing::instrument(skip_all)]
    pub fn build(self, config: &Config) -> Toolset {
        let mut toolset = Toolset::default().with_plugins(config.plugins.clone());
        load_config_files(config, &mut toolset);
        load_runtime_env(&mut toolset, env::vars().collect());
        load_runtime_args(&mut toolset, &self.args);
        toolset.resolve(config);

        if self.install_missing {
            if let Err(e) = toolset.install_missing(config) {
                warn!("Error installing runtimes: {}", e);
            };
        }

        debug!("{}", toolset);
        toolset
    }
}

fn load_config_files(config: &Config, ts: &mut Toolset) {
    let toolsets: Vec<_> = config
        .config_files
        .values()
        .collect_vec()
        .into_par_iter()
        .rev()
        .map(|cf| cf.to_toolset())
        .collect();
    for toolset in toolsets {
        ts.merge(toolset);
    }
}

fn load_runtime_env(ts: &mut Toolset, env: IndexMap<String, String>) {
    for (k, v) in env {
        if k.starts_with("RTX_") && k.ends_with("_VERSION") {
            let plugin_name = k[4..k.len() - 8].to_lowercase();
            let source = ToolSource::Environment(k, v.clone());
            let mut env_ts = Toolset::new(source);
            let version = ToolVersion::new(plugin_name.clone(), ToolVersionType::Version(v));
            env_ts.add_version(plugin_name, version);
            ts.merge(env_ts);
        }
    }
}

fn load_runtime_args(ts: &mut Toolset, args: &[RuntimeArg]) {
    for (plugin_name, args) in args.iter().into_group_map_by(|arg| arg.plugin.clone()) {
        let mut arg_ts = Toolset::new(ToolSource::Argument);
        for arg in args {
            match arg.version {
                RuntimeArgVersion::Version(ref v) => {
                    let version =
                        ToolVersion::new(plugin_name.clone(), ToolVersionType::Version(v.clone()));
                    arg_ts.add_version(plugin_name.clone(), version);
                }
                RuntimeArgVersion::Ref(ref v) => {
                    let version =
                        ToolVersion::new(plugin_name.clone(), ToolVersionType::Ref(v.clone()));
                    arg_ts.add_version(plugin_name.clone(), version);
                }
                RuntimeArgVersion::Path(ref v) => {
                    let version =
                        ToolVersion::new(plugin_name.clone(), ToolVersionType::Path(v.clone()));
                    arg_ts.add_version(plugin_name.clone(), version);
                }
                RuntimeArgVersion::Prefix(ref v) => {
                    let version =
                        ToolVersion::new(plugin_name.clone(), ToolVersionType::Prefix(v.clone()));
                    arg_ts.add_version(plugin_name.clone(), version);
                }
                // I believe this will do nothing since it would just default to the `.tool-versions` version
                // RuntimeArgVersion::None => {
                //     arg_ts.add_version(plugin_name.clone(), ToolVersion::None);
                // },
                _ => {
                    trace!("ignoring: {:?}", arg);
                }
            }
        }
        ts.merge(arg_ts);
    }
}
