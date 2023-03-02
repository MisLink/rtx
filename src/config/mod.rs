use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use color_eyre::eyre::{eyre, Result, WrapErr};
use color_eyre::Report;
use indexmap::IndexMap;
use itertools::Itertools;
use once_cell::sync::OnceCell;
use rayon::prelude::*;

pub use settings::{MissingRuntimeBehavior, Settings};

use crate::config::config_file::legacy_version::LegacyVersionFile;
use crate::config::config_file::rtxrc::RTXFile;
use crate::config::config_file::ConfigFile;
use crate::plugins::{Plugin, PluginName};
use crate::shorthands::{get_shorthands, Shorthands};
use crate::{dirs, env, file};

pub mod config_file;
mod settings;

type AliasMap = IndexMap<PluginName, IndexMap<String, String>>;

#[derive(Debug, Default)]
pub struct Config {
    pub settings: Settings,
    pub rtxrc: RTXFile,
    pub legacy_files: IndexMap<String, PluginName>,
    pub config_files: IndexMap<PathBuf, Box<dyn ConfigFile>>,
    pub aliases: AliasMap,
    pub plugins: IndexMap<PluginName, Arc<Plugin>>,
    pub env: IndexMap<String, String>,
    shorthands: OnceCell<HashMap<String, String>>,
}

impl Config {
    #[tracing::instrument]
    pub fn load() -> Result<Self> {
        let plugins = load_plugins()?;
        let rtxrc = load_rtxrc()?;
        let mut settings = rtxrc.settings();
        let config_files = load_all_config_files(
            &settings.build(),
            &plugins,
            &IndexMap::new(),
            IndexMap::new(),
        );
        for cf in config_files.values() {
            settings.merge(cf.settings());
        }
        let settings = settings.build();
        let legacy_files = load_legacy_files(&settings, &plugins);
        let config_files = load_all_config_files(&settings, &plugins, &legacy_files, config_files);
        let env = load_env(&config_files);
        let aliases = load_aliases(&settings, &plugins, &config_files);

        let config = Self {
            settings,
            legacy_files,
            config_files,
            aliases,
            rtxrc,
            plugins,
            env,
            shorthands: OnceCell::new(),
        };

        debug!("{}", &config);

        Ok(config)
    }

    pub fn get_shorthands(&self) -> &Shorthands {
        self.shorthands
            .get_or_init(|| get_shorthands(&self.settings))
    }

    pub fn is_activated(&self) -> bool {
        env::var("__RTX_DIFF").is_ok()
    }
}

fn load_rtxrc() -> Result<RTXFile> {
    let settings_path = dirs::CONFIG.join("config.toml");
    let rtxrc = if !settings_path.exists() {
        trace!("settings does not exist {:?}", settings_path);
        RTXFile::init(&settings_path)
    } else {
        let rtxrc = RTXFile::from_file(&settings_path)
            .wrap_err_with(|| err_load_settings(&settings_path))?;
        trace!("Settings: {:#?}", rtxrc.settings());
        rtxrc
    };

    Ok(rtxrc)
}

fn load_plugins() -> Result<IndexMap<PluginName, Arc<Plugin>>> {
    let plugins = Plugin::list()?
        .into_par_iter()
        .map(|p| (p.name.clone(), Arc::new(p)))
        .collect::<Vec<_>>()
        .into_iter()
        .sorted_by_cached_key(|(p, _)| p.to_string())
        .collect();
    Ok(plugins)
}

fn load_legacy_files(
    settings: &Settings,
    plugins: &IndexMap<PluginName, Arc<Plugin>>,
) -> IndexMap<String, PluginName> {
    if !settings.legacy_version_file {
        return IndexMap::new();
    }
    plugins
        .values()
        .collect_vec()
        .into_par_iter()
        .filter_map(|plugin| match plugin.legacy_filenames(settings) {
            Ok(filenames) => Some(
                filenames
                    .iter()
                    .map(|f| (f.to_string(), plugin.name.clone()))
                    .collect_vec(),
            ),
            Err(err) => {
                eprintln!("Error: {err}");
                None
            }
        })
        .collect::<Vec<Vec<(String, PluginName)>>>()
        .into_iter()
        .flatten()
        .collect()
}

fn load_all_config_files(
    settings: &Settings,
    plugins: &IndexMap<PluginName, Arc<Plugin>>,
    legacy_filenames: &IndexMap<String, PluginName>,
    mut existing: IndexMap<PathBuf, Box<dyn ConfigFile>>,
) -> IndexMap<PathBuf, Box<dyn ConfigFile>> {
    let mut filenames = vec![
        env::RTX_DEFAULT_CONFIG_FILENAME.as_str(),
        env::RTX_DEFAULT_TOOL_VERSIONS_FILENAME.as_str(),
    ];
    for filename in legacy_filenames.keys() {
        filenames.push(filename.as_str());
    }
    filenames.reverse();

    let mut config_files = file::FindUp::new(&dirs::CURRENT, &filenames).collect::<Vec<_>>();

    let home_config = dirs::HOME.join(env::RTX_DEFAULT_TOOL_VERSIONS_FILENAME.as_str());
    if home_config.is_file() {
        config_files.push(home_config);
    }

    config_files
        .into_iter()
        .unique()
        .map(|f| (f.clone(), existing.shift_remove(&f)))
        .collect_vec()
        .into_par_iter()
        .map(|(f, existing)| match existing {
            // already parsed so just return it
            Some(cf) => Some((f, cf)),
            // need to parse this config file
            None => match parse_config_file(&f, settings, legacy_filenames, plugins) {
                Ok(cf) => Some((f, cf)),
                Err(err) => {
                    warn!("error parsing: {} {err}", f.display());
                    None
                }
            },
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flatten()
        .collect()
}

fn parse_config_file(
    f: &PathBuf,
    settings: &Settings,
    legacy_filenames: &IndexMap<String, PluginName>,
    plugins: &IndexMap<PluginName, Arc<Plugin>>,
) -> Result<Box<dyn ConfigFile>> {
    match legacy_filenames.get(&f.file_name().unwrap().to_string_lossy().to_string()) {
        Some(plugin) => LegacyVersionFile::parse(settings, f.into(), plugins.get(plugin).unwrap())
            .map(|f| Box::new(f) as Box<dyn ConfigFile>),
        None => config_file::parse(f),
    }
}

fn load_env(config_files: &IndexMap<PathBuf, Box<dyn ConfigFile>>) -> IndexMap<String, String> {
    let mut env = IndexMap::new();
    for cf in config_files.values() {
        env.extend(cf.env());
    }
    env
}

fn load_aliases(
    settings: &Settings,
    plugins: &IndexMap<PluginName, Arc<Plugin>>,
    config_files: &IndexMap<PathBuf, Box<dyn ConfigFile>>,
) -> AliasMap {
    let mut aliases: AliasMap = IndexMap::new();
    let plugin_aliases: Vec<_> = plugins
        .values()
        .par_bridge()
        .map(|plugin| {
            let aliases = match plugin.get_aliases(settings) {
                Ok(aliases) => aliases,
                Err(err) => {
                    eprintln!("Error: {err}");
                    IndexMap::new()
                }
            };
            (&plugin.name, aliases)
        })
        .collect();
    for (plugin, plugin_aliases) in plugin_aliases {
        for (from, to) in plugin_aliases {
            aliases
                .entry(plugin.clone())
                .or_insert_with(IndexMap::new)
                .insert(from, to);
        }
    }

    for config_file in config_files.values() {
        for (plugin, plugin_aliases) in config_file.aliases() {
            for (from, to) in plugin_aliases {
                aliases
                    .entry(plugin.clone())
                    .or_insert_with(IndexMap::new)
                    .insert(from, to);
            }
        }
    }

    for (plugin, plugin_aliases) in &settings.aliases {
        for (from, to) in plugin_aliases {
            aliases
                .entry(plugin.clone())
                .or_insert_with(IndexMap::new)
                .insert(from.clone(), to.clone());
        }
    }

    aliases
}

fn err_load_settings(settings_path: &Path) -> Report {
    eyre!(
        "error loading settings from {}",
        settings_path.to_string_lossy()
    )
}

impl Display for Config {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let plugins = self
            .plugins
            .keys()
            .map(|p| p.to_string())
            .collect::<Vec<_>>();
        let config_files = self
            .config_files
            .iter()
            .map(|(p, _)| {
                p.to_string_lossy()
                    .to_string()
                    .replace(&dirs::HOME.to_string_lossy().to_string(), "~")
            })
            .collect::<Vec<_>>();
        writeln!(f, "Files: {}", config_files.join(", "))?;
        write!(f, "Installed Plugins: {}", plugins.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_display_snapshot;

    use super::*;

    #[test]
    fn test_load() {
        let config = Config::load().unwrap();
        assert_display_snapshot!(config);
    }
}
