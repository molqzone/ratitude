param(
  [string]$Device = "STM32F407ZG",
  [string]$Interface = "SWD",
  [int]$Speed = 4000,
  [int]$RttTelnetPort = 19021,
  [string]$Serial = "",
  [string]$Ip = ""
)

Write-Host ("Starting J-Link RTT backend: device={0}, if={1}, speed={2}, rtt_port={3}" -f $Device, $Interface, $Speed, $RttTelnetPort)

$args = @(
  "-if", $Interface,
  "-speed", $Speed,
  "-device", $Device,
  "-RTTTelnetPort", $RttTelnetPort,
  "-silent",
  "-singlerun"
)

if (-not [string]::IsNullOrWhiteSpace($Serial)) {
  $args += @("-USB", $Serial)
} elseif (-not [string]::IsNullOrWhiteSpace($Ip)) {
  $args += @("-IP", $Ip)
}

& JLinkGDBServerCLExe @args
$code = $LASTEXITCODE
if ($code -ne 0) {
  Write-Error ("J-Link backend exited with code {0}. Please check target power, SWD wiring, and device name." -f $code)
  exit $code
}
