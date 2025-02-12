# Cargo Regression

[![ci](https://github.com/zao111222333/cargo-regression/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/zao111222333/cargo-regression/actions/workflows/ci.yml)
[![crates.io](https://shields.io/crates/v/cargo-regression.svg?style=flat-square&label=crates.io)](https://crates.io/crates/cargo-regression)
[![License](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)

Collect test task from input files, execute them and compare results with golden.

## Usage
``` shell
cargo install cargo-regression
```
Build your test files like [./demo](./demo), then
``` shell
cargo regression ./demo --debug
```
![](screenshot.svg)

The tests will be exectued in `./tmp` in default, change the dir by `--work-dir path`.

### Set Extension(s)
`cargo-regression` will collect all files that match extensions as test tasks, you can set extensions in two ways:
+ By commmand arguments
``` shell
cargo regression ./demo --extensions py sh
```
+ By `__all__.toml`
``` toml
# override for all sub-dir
extensions = ["py", "sh"]
```

And the arguments `--extensions` is equivalent to set it in the most top `__all__.toml`.

### Other Configurations
There are many other configs that hold the same behavior as `extensions`:
| Argument | `__all__.toml` | Description |
| -- | -- | -- |
| `--exe-path bash` | `exe-path = "bash"` | The executable path to execute task |
| `--args {{name}}.sh arg1` | `args = ["{{name}}.sh", "arg1"]` | The arguements for execute task |
| `--permits 2` | `permits = 2` | The total permits to limit max parallelism |
| NA | `ignore = true` | Ignore that task |

### Variable Table
| Variable | Description |
| -- | -- |
| `{{root-dir}}`  | The absolute path of test root. |
| `{{name}}`      | The name of task file. |
| `{{extension}}` | The extension of task file. |

## Advanced Features
### Test Filter
Only test specified tasks.
``` shell
cargo regression ./demo
cargo regression ./demo --filter demo/trybuild/*
```

### Schedule Parallelism
`permits` and `permit` are virtual resource costs, you can define `permits` in arguments (default=1), and define `permit` in task toml config file (default=0).
``` shell
cargo regression ./demo --filter demo/test-premit/* --permits 1
cargo regression ./demo --filter demo/test-premit/* --permits 2
```


## assertion

### `exit-code`

Assert the exit code, default is `0`.
``` toml
[assert]
exit-code = 1
```

### `equal`
The output file should equal to the golden
``` toml
[assert]
equal = true
```

### `match`

Match pattern and assert the number (count) of it.
``` toml
[[assert.golden]]
file = "{{name}}.stdout"
match = [
  # regular expression match
  { pattern = 'f.*o', count = 4 },
  # should contain word "fo" at least once
  { pattern = '\bfo\b', count-at-least = 1 },
  # should contain word "fo" at most once
  { pattern = '\bfo0\b', count-at-most = 1 },
]
```

## Use it as API
see [./examples](./examples)

``` rust
use cargo_regression::{test, Args, TestExitCode};

#[tokio::main]
async fn main() -> TestExitCode {
  // Get arguments from CLI
  let args = Args::parse_from(std::env::args_os());
  // Or set fixed arguemnts
  let args = Args::new().debug(true).work_dir("tmp").root_dir("demo");
  test(args).await
}
```

## TODO
+ regolden
+ assert value
+ full config
+ document
