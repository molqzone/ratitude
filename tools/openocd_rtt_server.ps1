param(
  [string]$Elf = "d:/Repos/ratitude/build/stm32f4_rtt/stm32f4_rtt.elf",
  [string]$Interface = "interface/cmsis-dap.cfg",
  [string]$Target = "target/stm32f4x.cfg",
  [string]$Transport = "swd",
  [int]$Port = 19021,
  [int]$Polling = 1,
  [int]$Speed = 8000,
  [bool]$DisableDebugPorts = $true
)

$nmCmd = Get-Command arm-none-eabi-nm -ErrorAction SilentlyContinue
if ($null -eq $nmCmd) {
  Write-Error "arm-none-eabi-nm not found in PATH."
  exit 1
}

$elfCandidates = @()
if (Test-Path -LiteralPath $Elf) {
  $elfCandidates += $Elf
}

$nmLine = $null
$elfPath = $null
foreach ($candidate in $elfCandidates) {
  $line = & $nmCmd.Source -S $candidate 2>$null | Select-String -Pattern "_SEGGER_RTT" | Select-Object -First 1
  if ($null -ne $line) {
    $nmLine = $line
    $elfPath = $candidate
    break
  }
}

if ($null -eq $nmLine) {
  $found = $false
  Get-ChildItem -Recurse -File -Filter "*.elf" | ForEach-Object {
    if ($found) { return }
    $line = & $nmCmd.Source -S $_.FullName 2>$null | Select-String -Pattern "_SEGGER_RTT" | Select-Object -First 1
    if ($null -ne $line) {
      $nmLine = $line
      $elfPath = $_.FullName
      $found = $true
    }
  }
}

if ($null -eq $nmLine) {
  Write-Error "Failed to locate _SEGGER_RTT in any ELF. Check build output path."
  exit 1
}

if ($nmLine.Line -match '^(?<addr>[0-9A-Fa-f]+)\s+(?<size>[0-9A-Fa-f]+)\s+\w\s+_SEGGER_RTT') {
  $addr = [Convert]::ToUInt32($Matches['addr'], 16)
  $size = [Convert]::ToUInt32($Matches['size'], 16)
} else {
  Write-Error "Unexpected nm output: $($nmLine.Line)"
  exit 1
}

$addrHex = ("0x{0:X}" -f $addr)
$sizeHex = ("0x{0:X}" -f $size)

Write-Host ("Using ELF: {0}" -f $elfPath)
Write-Host ("_SEGGER_RTT at {0}, size {1}" -f $addrHex, $sizeHex)
Write-Host ("RTT polling_interval: {0} ms, adapter speed: {1} kHz" -f $Polling, $Speed)
Write-Host ("Debug ports disabled: {0}" -f $DisableDebugPorts)

$openocdArgs = @(
  "-f", $Interface,
  "-f", $Target,
  "-c", ("transport select {0}" -f $Transport),
  "-c", ("adapter speed {0}" -f $Speed),
  "-c", "init",
  "-c", "reset run",
  "-c", ('rtt setup {0} {1} "SEGGER RTT"' -f $addrHex, $sizeHex),
  "-c", ("rtt polling_interval {0}" -f $Polling),
  "-c", "rtt start",
  "-c", "resume",
  "-c", ("rtt server start {0} 0" -f $Port)
)

if ($DisableDebugPorts) {
  $openocdArgs = @(
    "-f", $Interface,
    "-f", $Target,
    "-c", ("transport select {0}" -f $Transport),
    "-c", ("adapter speed {0}" -f $Speed),
    "-c", "gdb_port disabled",
    "-c", "tcl_port disabled",
    "-c", "telnet_port disabled",
    "-c", "init",
    "-c", "reset run",
    "-c", ('rtt setup {0} {1} "SEGGER RTT"' -f $addrHex, $sizeHex),
    "-c", ("rtt polling_interval {0}" -f $Polling),
    "-c", "rtt start",
    "-c", "resume",
    "-c", ("rtt server start {0} 0" -f $Port)
  )
}

& openocd @openocdArgs
