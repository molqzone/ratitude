param(
    [string]$Config = "examples/mock/rat.toml"
)

Write-Error @"
run_mock_foxglove.ps1 is removed and no longer supported.
Mock RTT pipeline was decommissioned.
Use a real RTT endpoint, then run:
  cargo run -p ratsync -- --config <path/to/rat.toml>
  cargo run -p ratd -- --config <path/to/rat.toml>
"@
exit 1
