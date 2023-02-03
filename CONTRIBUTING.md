## Dependencies

* [rust](https://www.rust-lang.org/) stable 1.66.1+ (it might be compatible with earlier, but I haven't tested that). As of this writing: 1.67.0 but GH actions will use the latest stable whenever it runs.
* [just](https://github.com/casey/just) any version should do, but as of this writing I'm on 1.13.0

(you'd think we'd use rtx to fetch these but frankly it's kind of a pain to dogfood rtx while testing it)

## Setup

Shouldn't require anything special I'm aware of, but `just build` is a good sanity check to run and make sure it's all working.

## Running the CLI

I put a shim for `cargo run` that makes it easy to run build + run rtx in dev mode. It's at `.bin/rtx`. What I do is add this to PATH
with direnv. Here is my `.envrc`:

```
source_up_if_exists
PATH_add .bin
```

Now I can just run `rtx` as if I was using an installed version and it will build it from source everytime there are changes.

You don't have to do this, but it makes things like `rtx activate` a lot easier to setup.

## Running Tests

* Run only unit tests: `just test-unit`
* Run only E2E tests: `just test-e2e`
* Run all tests: `just test`

## Linting

* Lint codebase: `just lint`
* Lint and fix codebase: `just lint-fix`

## Generating readme and shell completion files

```
just pre-commit
```

## [optional] Pre-commit hook

This project uses husky which will automatically install a pre-commit hook:

```
npm i # installs and configured husky precommit hook automatically
git commit # will automatically run `just pre-commit`
```