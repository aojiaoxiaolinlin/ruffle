[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$ExecutablePath,
    [ValidateSet(0, 1)][int]$GcWorkFloor = 0,
    [switch]$ValidateOnly
)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSEdition -ne 'Desktop' -or -not [Environment]::Is64BitProcess) {
    throw 'Use 64-bit Windows PowerShell 5.1.'
}
$executable = [IO.Path]::GetFullPath($ExecutablePath)
$executableDirectory = Split-Path -Parent $executable
$identityPath = Join-Path $executableDirectory 'build-identity.json'
$identity = Get-Content -LiteralPath $identityPath -Raw | ConvertFrom-Json
if ($GcWorkFloor -eq 1 -and -not $identity.gc_work_floor_supported) {
    throw 'This build identity does not declare GC work floor support.'
}
$algorithm = [Security.Cryptography.SHA256]::Create()
$stream = [IO.File]::OpenRead($executable)
try { $hash = [BitConverter]::ToString($algorithm.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
finally { $stream.Dispose(); $algorithm.Dispose() }
if ($hash -ne $identity.exe_sha256) { throw 'EXE identity mismatch. Rebuild or restore the paired artifacts.' }
if (-not (Test-Path -LiteralPath (Join-Path $executableDirectory 'ruffle_desktop.pdb'))) { throw 'Matching PDB missing.' }
if ($ValidateOnly) {
    Write-Host ('Ready: ' + $identity.revision + ' / ' + $hash)
    Write-Host 'Validation only; no game launched.'
    return
}

$session = Join-Path (Join-Path $executableDirectory 'sessions') (Get-Date -Format 'yyyyMMdd-HHmmss-fff')
New-Item -ItemType Directory -Path $session -Force | Out-Null
$cache = Join-Path $session 'game-cache'
$logFilter = 'warn,ruffle_desktop=info,ruffle_core::performance_diagnostics=info,ruffle_core::library=info,ruffle_render_wgpu=info'
$oldLog = [Environment]::GetEnvironmentVariable('RUST_LOG', 'Process')
$oldPolicy = [Environment]::GetEnvironmentVariable('RUFFLE_DIAGNOSTIC_SWF_CACHE_IDLE_SECONDS', 'Process')
$oldGcFloor = [Environment]::GetEnvironmentVariable('RUFFLE_DIAGNOSTIC_GC_WORK_FLOOR', 'Process')
$oldBacktrace = [Environment]::GetEnvironmentVariable('RUST_BACKTRACE', 'Process')
try {
    $env:RUST_LOG = $logFilter
    $env:RUFFLE_DIAGNOSTIC_SWF_CACHE_IDLE_SECONDS = '0'
    $env:RUFFLE_DIAGNOSTIC_GC_WORK_FLOOR = [string]$GcWorkFloor
    $env:RUST_BACKTRACE = '1'
    $game = Start-Process -FilePath $executable -WorkingDirectory $executableDirectory -WindowStyle Normal -PassThru `
        -ArgumentList @('--cache-directory', ('"' + $cache + '"')) `
        -RedirectStandardOutput (Join-Path $session 'stdout.log') `
        -RedirectStandardError (Join-Path $session 'stderr.log')
    # Keep a native handle before exit; Start-Process can otherwise leave the
    # eventual ExitCode unavailable after the short-lived process is gone.
    $null = $game.Handle
}
finally {
    [Environment]::SetEnvironmentVariable('RUST_LOG', $oldLog, 'Process')
    [Environment]::SetEnvironmentVariable('RUFFLE_DIAGNOSTIC_SWF_CACHE_IDLE_SECONDS', $oldPolicy, 'Process')
    [Environment]::SetEnvironmentVariable('RUFFLE_DIAGNOSTIC_GC_WORK_FLOOR', $oldGcFloor, 'Process')
    [Environment]::SetEnvironmentVariable('RUST_BACKTRACE', $oldBacktrace, 'Process')
}
@{
    revision = $identity.revision
    sha256 = $hash
    pid = $game.Id
    started_utc = [DateTime]::UtcNow.ToString('o')
    exe = $executable
    rust_log = $logFilter
    render_cache_idle_seconds = 0
    gc_work_floor_enabled = $GcWorkFloor
    rust_backtrace = '1'
    sampling = 'continuous process totals; includes background time; no automatic battle/idle classification'
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $session 'session.json') -Encoding UTF8

Write-Host ('Test build: ' + $identity.revision)
Write-Host ('GC work floor: ' + $GcWorkFloor + ' (0=baseline, 1=experimental).')
Write-Host ('Output: ' + $session)
Write-Host 'Sampling automatically. No hotkeys needed. Keep this console open.'
Write-Host 'Close the game normally when finished; sampling then stops. Existing games are not touched.'
$writer = [IO.StreamWriter]::new((Join-Path $session 'os-memory.csv'), $false, [Text.UTF8Encoding]::new($false))
$writer.AutoFlush = $true
$writer.WriteLine('utc,pid,private_commit_bytes,working_set_bytes,virtual_memory_bytes')
$systemWriter = [IO.StreamWriter]::new((Join-Path $session 'system-memory.csv'), $false, [Text.UTF8Encoding]::new($false))
$systemWriter.AutoFlush = $true
$systemWriter.WriteLine('utc,committed_bytes,commit_limit_bytes,available_physical_MiB,status')
$nextSystemSample = [DateTime]::MinValue
try {
    while ($true) {
        $game.Refresh()
        if ($game.HasExited) { break }
        if ([DateTime]::UtcNow -ge $nextSystemSample) {
            try {
                $systemMemory = Get-CimInstance Win32_PerfFormattedData_PerfOS_Memory -ErrorAction Stop
                $systemWriter.WriteLine(('{0},{1},{2},{3},ok' -f [DateTime]::UtcNow.ToString('o'),
                    $systemMemory.CommittedBytes, $systemMemory.CommitLimit, $systemMemory.AvailableMBytes))
            }
            catch {
                $systemWriter.WriteLine(([DateTime]::UtcNow.ToString('o') + ',,,,unavailable'))
            }
            $nextSystemSample = [DateTime]::UtcNow.AddSeconds(5)
        }
        try {
            $writer.WriteLine(('{0},{1},{2},{3},{4}' -f [DateTime]::UtcNow.ToString('o'), $game.Id,
                $game.PrivateMemorySize64, $game.WorkingSet64, $game.VirtualMemorySize64))
        }
        catch {
            $game.Refresh()
            if ($game.HasExited) { break }
            throw
        }
        Start-Sleep -Seconds 1
    }
    $null = $game.WaitForExit(1000)
    @{
        ended_utc = [DateTime]::UtcNow.ToString('o')
        outcome = 'process_exited'
        exit_code = $game.ExitCode
    } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $session 'result.json') -Encoding UTF8
    Write-Host ('Sampling saved: ' + $session)
}
finally {
    $writer.Dispose()
    $systemWriter.Dispose()
    # Never terminate the game when sampling fails or the console is closed.
}
