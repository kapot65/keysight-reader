# Keysight reader


## Virtual mode
To run virtual mode either^
- run cargo with `--features virtual`
  ```shell
  RUST_LOG=info cargo run --release --bin keithley-6430-reader --features virtual
  ```
- set virtual feature as default in `Cargo.toml`
  ```toml
  [features]
  #...
  default = ["virtual"]
  ```

## requirements
libgpib must be installed and configured properly. See [keythley-loader](https://github.com/kapot65/keithley-loader) for instructions.