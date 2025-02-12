# Cargo Regression

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
| `{{root_dir}}`  | The absolute path of test root. |
| `{{name}}`      | The name of task file. |
| `{{extension}}` | The extension of task file. |

## Advanced Features
### Test Filter
Only test specified tasks.
``` shell
cargo regression ./demo
cargo regression ./demo --filter demo/test_premit/*
```

### Schedule Parallelism
`permits` and `permit` are virtual resource costs, you can define `permits` in arguments (default=1), and define `permit` in task toml config file (default=0).
``` shell
cargo regression ./demo --filter demo/test_premit/*
cargo regression ./demo --filter demo/test_premit/* --permits 2
```


## assertion

### `exit_code`

Assert the exit code, default is `0`.
``` toml
[assert]
exit_code = 1
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
  # regular expression to match "fo", "foo", "fooo", ...
  { pattern = "f.*o", count = 4 },
  # this means file should contain "fo"
  { pattern = "fo", count_at_least = 1 },
]
```

## TODO
+ regolden
+ assert value
+ full config
+ document
