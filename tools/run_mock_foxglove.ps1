param(
    [string]$Config = "examples/mock/rat.toml",
    [string]$Host = "127.0.0.1",
    [int]$Port = 19021
)

$repoRoot = (Resolve-Path (Join-Path "${PSScriptRoot}" "..")).Path
$configPath = $Config
if (-not [System.IO.Path]::IsPathRooted($configPath)) {
    $configPath = Join-Path $repoRoot $configPath
}
$configPath = (Resolve-Path $configPath).Path
$mockScript = Join-Path $repoRoot "tools/openocd_rtt_mock.py"

Push-Location $repoRoot

$mockProc = $null
try {
    $mockArgs = @(
        "-X", "utf8",
        $mockScript,
        "--config", $configPath,
        "--host", $Host,
        "--port", $Port,
        "--profile", "balanced"
    )
    $mockProc = Start-Process -FilePath "python" -ArgumentList $mockArgs -PassThru -NoNewWindow

    Start-Sleep -Milliseconds 300

    cargo run -p ratd -- --config "$configPath"
    exit $LASTEXITCODE
}
finally {
    if ($null -ne $mockProc -and -not $mockProc.HasExited) {
        Stop-Process -Id $mockProc.Id -Force -ErrorAction SilentlyContinue
        Wait-Process -Id $mockProc.Id -ErrorAction SilentlyContinue
    }
    Pop-Location
}
