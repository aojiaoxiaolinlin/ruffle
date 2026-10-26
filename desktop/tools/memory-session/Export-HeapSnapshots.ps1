[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$SessionDirectory)
$ErrorActionPreference = 'Stop'
$directory = (Resolve-Path -LiteralPath $SessionDirectory).Path
$trace = Join-Path $directory 'heap-snapshots.etl'
if (-not (Test-Path -LiteralPath $trace)) { throw "No heap trace found: $trace" }
$oldSymbols = [Environment]::GetEnvironmentVariable('_NT_SYMBOL_PATH', 'Process')
$oldCache = [Environment]::GetEnvironmentVariable('_NT_SYMCACHE_PATH', 'Process')
try {
    # Keep matching application PDBs beside the ETL; no symbol-server download.
    $env:_NT_SYMBOL_PATH = $directory
    $env:_NT_SYMCACHE_PATH = Join-Path $directory 'symcache'
    & xperf -i $trace -symbols -o (Join-Path $directory 'heap-export.txt') -a heapsnapshot -instance -data -stack -count
    if ($LASTEXITCODE -ne 0) { throw 'xperf heap export failed.' }
    & xperf -i $trace -o (Join-Path $directory 'trace-stats.txt') -a tracestats -detail
    if ($LASTEXITCODE -ne 0) { throw 'xperf trace statistics failed.' }
}
finally {
    [Environment]::SetEnvironmentVariable('_NT_SYMBOL_PATH', $oldSymbols, 'Process')
    [Environment]::SetEnvironmentVariable('_NT_SYMCACHE_PATH', $oldCache, 'Process')
}
Write-Host "Heap export completed: $directory"
