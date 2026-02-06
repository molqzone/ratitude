param(
  [string]$OpenOcdLog = "d:/Repos/ratitude/openocd_rtt.err.log",
  [string]$Jsonl = "d:/Repos/ratitude/out.jsonl",
  [int]$Port = 19021
)

function Write-Section($title) {
  Write-Host ""
  Write-Host ("== {0} ==" -f $title)
}

function Get-OpenOcdProcesses {
  try {
    return Get-Process -Name openocd -ErrorAction Stop
  } catch {
    return @()
  }
}

function Get-ListeningPort($port) {
  $connections = @()
  $cmd = Get-Command Get-NetTCPConnection -ErrorAction SilentlyContinue
  if ($null -ne $cmd) {
    try {
      $connections = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction Stop
    } catch {
      $connections = @()
    }
  } else {
    $netstat = netstat -ano | Select-String -Pattern (":" + $port + "\s")
    if ($netstat) {
      $connections = $netstat
    }
  }
  return $connections
}

function Parse-JsonlStats($path) {
  if (-not (Test-Path -LiteralPath $path)) {
    return $null
  }
  $lines = Get-Content -LiteralPath $path -Tail 200 -ErrorAction SilentlyContinue
  if (-not $lines) {
    return $null
  }
  $timestamps = New-Object System.Collections.Generic.List[DateTimeOffset]
  foreach ($line in $lines) {
    try {
      $obj = $line | ConvertFrom-Json
    } catch {
      continue
    }
    if ($null -eq $obj.ts) { continue }
    try {
      $dt = [DateTimeOffset]::Parse($obj.ts)
    } catch {
      continue
    }
    $timestamps.Add($dt)
  }
  if ($timestamps.Count -lt 2) {
    return $null
  }
  $deltas = New-Object System.Collections.Generic.List[double]
  for ($i = 1; $i -lt $timestamps.Count; $i++) {
    $delta = ($timestamps[$i] - $timestamps[$i - 1]).TotalSeconds
    if ($delta -ge 0) {
      $deltas.Add($delta)
    }
  }
  if ($deltas.Count -eq 0) {
    return $null
  }
  $sorted = $deltas | Sort-Object
  $p50 = $sorted[[int]($sorted.Count * 0.5)]
  $p90 = $sorted[[int]($sorted.Count * 0.9)]
  $p99 = $sorted[[int]($sorted.Count * 0.99)]
  return [pscustomobject]@{
    Count = $deltas.Count
    P50 = $p50
    P90 = $p90
    P99 = $p99
    Rate = if ($p50 -gt 0) { [Math]::Round(1.0 / $p50, 2) } else { 0 }
  }
}

Write-Section "OpenOCD Processes"
$procs = Get-OpenOcdProcesses
if ($procs.Count -eq 0) {
  Write-Host "OpenOCD not running"
} else {
  foreach ($p in $procs) {
    Write-Host ("PID={0} Start={1} Path={2}" -f $p.Id, $p.StartTime, $p.Path)
  }
  if ($procs.Count -gt 1) {
    Write-Host ("Warning: {0} OpenOCD processes detected; RTT may flap" -f $procs.Count)
  }
}

Write-Section "Port Listener"
$listeners = Get-ListeningPort $Port
if ($listeners.Count -eq 0) {
  Write-Host ("Port {0} is not listening" -f $Port)
} else {
  Write-Host ("Port {0} is listening" -f $Port)
}

Write-Section "OpenOCD Log"
if (Test-Path -LiteralPath $OpenOcdLog) {
  $tail = Get-Content -LiteralPath $OpenOcdLog -Tail 200 -ErrorAction SilentlyContinue
  $badFd = ($tail | Select-String -Pattern "Bad file descriptor").Count
  $dropped = ($tail | Select-String -Pattern "dropped 'rtt' connection").Count
  $halted = ($tail | Select-String -Pattern "halted due to debug-request").Count
  Write-Host ("Bad file descriptor: {0}" -f $badFd)
  Write-Host ("Dropped rtt connection: {0}" -f $dropped)
  Write-Host ("Target halted: {0}" -f $halted)
  $lastWrite = (Get-Item -LiteralPath $OpenOcdLog).LastWriteTime
  Write-Host ("Log last write: {0}" -f $lastWrite)
} else {
  Write-Host "openocd_rtt.err.log not found"
}

Write-Section "Data Rate (out.jsonl)"
$stats = Parse-JsonlStats $Jsonl
if ($null -eq $stats) {
  Write-Host "out.jsonl missing or not enough samples"
} else {
  Write-Host ("Samples: {0}" -f $stats.Count)
  Write-Host ("P50 interval: {0:n3}s (~ {1} samples/s)" -f $stats.P50, $stats.Rate)
  Write-Host ("P90 interval: {0:n3}s" -f $stats.P90)
  Write-Host ("P99 interval: {0:n3}s" -f $stats.P99)
}

Write-Section "Hints"
if ($procs.Count -gt 1) {
  Write-Host "- Multiple OpenOCD processes detected; stop extras"
}
if ($listeners.Count -eq 0) {
  Write-Host "- RTT port is not listening"
}
if ($badFd -gt 0 -or $dropped -gt 0) {
  Write-Host "- RTT connections are dropping; keep a single OpenOCD instance and disable debug ports"
}
if ($halted -gt 0) {
  Write-Host "- Target halted; ensure reset run/resume"
}
