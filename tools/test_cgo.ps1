param(
  [string]$Packages = "./...",
  [string]$CC = "gcc"
)

$ErrorActionPreference = "Stop"

function Resolve-CCompiler([string]$requested) {
  $cmd = Get-Command $requested -ErrorAction SilentlyContinue
  if ($null -ne $cmd) {
    return $requested
  }

  $mingwBin = Join-Path $env:USERPROFILE "scoop/apps/mingw/current/bin"
  if (Test-Path -LiteralPath $mingwBin) {
    $env:PATH = "${mingwBin};${env:PATH}"
    $cmd = Get-Command $requested -ErrorAction SilentlyContinue
    if ($null -ne $cmd) {
      return $requested
    }
    $gcc = Get-Command "gcc" -ErrorAction SilentlyContinue
    if ($null -ne $gcc) {
      return "gcc"
    }
  }

  throw "C compiler '${requested}' not found. Install MinGW (e.g. 'scoop install mingw') or provide -CC."
}

$resolvedCC = Resolve-CCompiler $CC

$previousCGO = $env:CGO_ENABLED
$previousCC = $env:CC

try {
  $env:CGO_ENABLED = "1"
  $env:CC = $resolvedCC

  Write-Host "[test-cgo] CGO_ENABLED=${env:CGO_ENABLED}" -ForegroundColor Cyan
  Write-Host "[test-cgo] CC=${env:CC}" -ForegroundColor Cyan
  Write-Host "[test-cgo] go test ${Packages}" -ForegroundColor Cyan

  & go test $Packages
  if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
  }

  Write-Host "[test-cgo] success" -ForegroundColor Green
}
finally {
  $env:CGO_ENABLED = $previousCGO
  $env:CC = $previousCC
}
