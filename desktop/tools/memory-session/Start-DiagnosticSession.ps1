[CmdletBinding()]
param(
    [ValidateRange(0, 86400)][int]$RenderCacheIdleSeconds = 0,
    [switch]$ValidateOnly
)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSEdition -ne 'Desktop' -or -not [Environment]::Is64BitProcess) {
    throw 'Use 64-bit Windows PowerShell 5.1 (powershell.exe).'
}
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$executable = Join-Path $repoRoot 'target\memory-diagnostics\release\ruffle_desktop.exe'
if (-not (Test-Path -LiteralPath $executable)) {
    throw 'Build first: cargo build -p ruffle_desktop --release --locked --features performance_diagnostics --target-dir target/memory-diagnostics'
}
# Verify the build features instead of assuming that a directory name proves
# diagnostics were enabled. Experimental memory policies must remain disabled.
$fingerprints = Join-Path $repoRoot 'target\memory-diagnostics\release\.fingerprint'
$diagnosticBuild = @(Get-ChildItem -LiteralPath $fingerprints -Directory -Filter 'ruffle_desktop-*' |
    ForEach-Object { Get-ChildItem -LiteralPath $_.FullName -Filter 'bin-ruffle_desktop.json' } |
    Where-Object {
        $features = (Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json).features | ConvertFrom-Json
        $features -contains 'performance_diagnostics' -and
            -not ($features -match 'experiment|xml_query_diagnostics')
    })
if ($diagnosticBuild.Count -eq 0) { throw 'No baseline performance_diagnostics build fingerprint found.' }
$hashAlgorithm = [Security.Cryptography.SHA256]::Create()
$executableStream = [IO.File]::OpenRead($executable)
try { $executableHash = [BitConverter]::ToString($hashAlgorithm.ComputeHash($executableStream)).Replace('-', '').ToLowerInvariant() }
finally { $executableStream.Dispose(); $hashAlgorithm.Dispose() }
$samplerScript = Join-Path $PSScriptRoot 'Start-MemorySession.ps1'
if ($ValidateOnly) {
    & $samplerScript -ValidateOnly
    Write-Host 'Diagnostic launcher paths and build features validated. No game launched.'
    Write-Host ('Requested SWF cache idle limit: ' + $RenderCacheIdleSeconds + ' seconds (0 = baseline).')
    return
}

$variant = if ($RenderCacheIdleSeconds -eq 0) { 'baseline' } else { 'idle-render-cache-candidate' }
$directory = Join-Path (Join-Path $PSScriptRoot 'sessions') ('resource-diagnostic-' + $variant + '-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
New-Item -ItemType Directory -Path $directory | Out-Null
$cache = Join-Path $directory 'game-cache'
$memory = Join-Path $directory 'memory'
$logFilter = 'warn,ruffle_desktop=info,ruffle_core::performance_diagnostics=info,ruffle_core::library=info,ruffle_render_wgpu=info'
$oldLogFilter = [Environment]::GetEnvironmentVariable('RUST_LOG', 'Process')
$cachePolicyVariable = 'RUFFLE_DIAGNOSTIC_SWF_CACHE_IDLE_SECONDS'
$oldCachePolicy = [Environment]::GetEnvironmentVariable($cachePolicyVariable, 'Process')
try {
    $env:RUST_LOG = $logFilter
    [Environment]::SetEnvironmentVariable($cachePolicyVariable, [string]$RenderCacheIdleSeconds, 'Process')
    # This is the interactive game window, which the user needs to see.
    # Existing games remain running; PID targeting keeps their samples separate.
    $gameProcess = Start-Process -FilePath $executable -ArgumentList @('--cache-directory', ('"' + $cache + '"')) `
        -WorkingDirectory $repoRoot -WindowStyle Normal -PassThru `
        -RedirectStandardOutput (Join-Path $directory 'game-stdout.log') `
        -RedirectStandardError (Join-Path $directory 'game-stderr.log')
}
finally {
    [Environment]::SetEnvironmentVariable('RUST_LOG', $oldLogFilter, 'Process')
    [Environment]::SetEnvironmentVariable($cachePolicyVariable, $oldCachePolicy, 'Process')
}
@(
    'started_utc=' + [DateTime]::UtcNow.ToString('o')
    'pid=' + $gameProcess.Id
    'exe=' + $executable
    'sha256=' + $executableHash
    'features=performance_diagnostics (plus package defaults); no extra experimental Cargo features'
    'swf_render_cache_idle_seconds=' + $RenderCacheIdleSeconds
    'variant=' + $variant
    'rust_log=' + $logFilter
    'cache=' + $cache
    'sampling=' + $memory
) | Set-Content -LiteralPath (Join-Path $directory 'diagnostic-build.txt') -Encoding UTF8
Write-Host ('Diagnostic session: ' + $directory)
Write-Host ('Regenerable SWF cache idle limit: ' + $RenderCacheIdleSeconds + ' seconds (0 = baseline).')
Write-Host ('New game PID: ' + $gameProcess.Id + '. Enter the game and wait for the map, then press F8.')
Write-Host 'F9: maps -> battles -> cooldown. F10: finish sampling. Switching apps does not stop observation.'
Write-Host 'Keep this PowerShell open. Close the diagnostic game normally after F10 to flush its logs.'
try {
    $null = $gameProcess.WaitForInputIdle(10000)
    if ($gameProcess.HasExited) { throw 'Diagnostic game exited. Inspect game-stderr.log in the session directory.' }
    & $samplerScript -Mode observe -ProcessId $gameProcess.Id -OutputDirectory $memory
    if (-not $?) { throw 'Memory sampler failed.' }
}
finally {
    # Never close the game on a sampler error; preserve the user's current play.
    $sampleBuild = Join-Path $memory 'build.txt'
    if (Test-Path -LiteralPath $sampleBuild) {
        @('variant=' + $variant; 'swf_render_cache_idle_seconds=' + $RenderCacheIdleSeconds) |
            Add-Content -LiteralPath $sampleBuild -Encoding UTF8
    }
    Write-Host ('Results: ' + $directory)
}
