[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateRange(1, 2147483647)][int]$TargetProcessId,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [ValidateRange(10, 600)][int]$CooldownSeconds = 120,
    [ValidateRange(1, 60)][int]$MaxMinutes = 15,
    [switch]$NotifyOnFinish,
    [switch]$ValidateOnly
)

$ErrorActionPreference = 'Stop'
$output = (Resolve-Path -LiteralPath $OutputDirectory).Path
$game = Get-Process -Id $TargetProcessId -ErrorAction Stop
$gamePath = $game.Path
if ([IO.Path]::GetFileName($gamePath) -ne 'ruffle_desktop.exe') { throw 'Target must be ruffle_desktop.exe.' }
$symbolDirectory = Split-Path -Parent $gamePath
if (-not (Test-Path -LiteralPath (Join-Path $symbolDirectory 'ruffle_desktop.pdb'))) {
    throw 'Matching application PDB must be beside the running executable.'
}
if (Test-Path -LiteralPath (Join-Path $output 'capture-status.txt')) { throw 'Use a fresh capture directory.' }
foreach ($name in @('baseline.command', 'after-battles.command', 'finish.command')) {
    if (Test-Path -LiteralPath (Join-Path $output $name)) { throw 'Capture directory contains old commands.' }
}
$null = Get-Command wpr.exe -ErrorAction Stop

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class GameHeapCaptureKeys {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int MessageBox(IntPtr owner, string text, string caption, uint flags);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    public static bool IsForeground(int expected) {
        uint actual;
        GetWindowThreadProcessId(GetForegroundWindow(), out actual);
        return actual == expected;
    }
}
'@

if ($ValidateOnly) {
    Write-Host "Validated target PID $TargetProcessId, PDB and control APIs. No WPR session started."
    return
}
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this capture helper in an administrator PowerShell; the game can stay unelevated.'
}

$instance = 'RuffleGameHeap-' + [Guid]::NewGuid().ToString('N')
$phase = 'starting'
$recording = $false
$configured = $false
$complete = $false
$stopSucceeded = $false
$failure = $null
$cooldownStarted = $null
$started = [DateTime]::UtcNow
$deadline = $started.AddMinutes($MaxMinutes)
$lastSample = [DateTime]::MinValue
$pressed = @{}

function Set-CapturePhase([string]$Value, [string]$Instruction) {
    $script:phase = $Value
    @('phase=' + $Value; 'pid=' + $TargetProcessId; 'updated_utc=' + [DateTime]::UtcNow.ToString('o');
      'instruction=' + $Instruction; 'instance=' + $instance) |
        Set-Content -LiteralPath (Join-Path $output 'capture-status.txt') -Encoding UTF8
    Write-Host ($Value + ': ' + $Instruction)
}
function Invoke-WprChecked([string[]]$Arguments) {
    $result = & "$env:WINDIR\System32\wpr.exe" @Arguments 2>&1
    $code = $LASTEXITCODE
    $result | Add-Content -LiteralPath (Join-Path $output 'wpr.log') -Encoding UTF8
    if ($code -ne 0) { throw "WPR failed ($code): $($Arguments -join ' ')" }
}
function Add-OsSample {
    $game.Refresh()
    if ($game.HasExited) { throw 'Game process exited; preserving partial capture.' }
    ([DateTime]::UtcNow.ToString('o') + ',' + $phase + ',' + $TargetProcessId + ',' +
        $game.PrivateMemorySize64 + ',' + $game.WorkingSet64 + ',' + $game.VirtualMemorySize64) |
        Add-Content -LiteralPath (Join-Path $output 'os-memory.csv') -Encoding UTF8
}
function Take-GameSnapshot([string]$Stage) {
    $game.Refresh()
    if ($game.HasExited) { throw 'Game exited before snapshot.' }
    $before = [DateTime]::UtcNow
    Invoke-WprChecked @('-singlesnapshot', 'heap', [string]$TargetProcessId, '-instancename', $instance)
    $after = [DateTime]::UtcNow
    ($Stage + "`t" + $TargetProcessId + "`t" + $before.ToString('o') + "`t" + $after.ToString('o') + "`t" +
        ($after - $before).TotalSeconds.ToString('F3', [Globalization.CultureInfo]::InvariantCulture)) |
        Add-Content -LiteralPath (Join-Path $output 'snapshots.tsv') -Encoding UTF8
    Add-OsSample
}
function Is-KeyPress([int]$Key) {
    $down = ([GameHeapCaptureKeys]::GetAsyncKeyState($Key) -band 0x8000) -ne 0
    $edge = $down -and -not $pressed[$Key]
    $pressed[$Key] = $down
    return $edge
}

try {
    foreach ($artifact in @($gamePath, (Join-Path $symbolDirectory 'ruffle_desktop.pdb'))) {
        $algorithm = [Security.Cryptography.SHA256]::Create()
        $stream = [IO.File]::OpenRead($artifact)
        try { $hash = [BitConverter]::ToString($algorithm.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
        finally { $stream.Dispose(); $algorithm.Dispose() }
        ([IO.Path]::GetFileName($artifact) + ' sha256=' + $hash) |
            Add-Content -LiteralPath (Join-Path $output 'artifacts.txt') -Encoding UTF8
        $savedArtifact = Join-Path $output ([IO.Path]::GetFileName($artifact))
        if ([IO.Path]::GetFullPath($artifact) -ne [IO.Path]::GetFullPath($savedArtifact)) {
            Copy-Item -LiteralPath $artifact -Destination $savedArtifact
        }
    }
    @('pid=' + $TargetProcessId; 'game_start_utc=' + $game.StartTime.ToUniversalTime().ToString('o');
      'exe=' + $gamePath; 'symbols=' + $symbolDirectory; 'instance=' + $instance;
      'tracking_started_utc=' + $started.ToString('o'); 'cooldown_seconds=' + $CooldownSeconds;
      'max_minutes=' + $MaxMinutes; 'scope=Windows heap; pre-enable allocations may lack application stacks') |
        Set-Content -LiteralPath (Join-Path $output 'capture-build.txt') -Encoding UTF8
    'utc,phase,pid,private_commit_bytes,working_set_bytes,virtual_memory_bytes' |
        Set-Content -LiteralPath (Join-Path $output 'os-memory.csv') -Encoding UTF8
    Invoke-WprChecked @('-start', 'HeapSnapshot', '-filemode', '-instancename', $instance)
    $recording = $true
    Invoke-WprChecked @('-snapshotconfig', 'heap', '-pid', [string]$TargetProcessId, 'enable')
    $configured = $true
    Set-CapturePhase 'armed' 'After login, stop on a map and press F8 in the game. Then battle a few times and press F9.'
    while ([DateTime]::UtcNow -lt $deadline) {
        $now = [DateTime]::UtcNow
        if (($now - $lastSample).TotalSeconds -ge 1) { Add-OsSample; $lastSample = $now }
        $f8 = Is-KeyPress 0x77
        $f9 = Is-KeyPress 0x78
        $f10 = Is-KeyPress 0x79
        $foreground = [GameHeapCaptureKeys]::IsForeground($TargetProcessId)
        if ($f10 -or (Test-Path -LiteralPath (Join-Path $output 'finish.command'))) {
            if ($phase -ne 'armed') { Take-GameSnapshot 'manual-finish' }
            Set-CapturePhase 'user_finished' 'Capture ended early; inspect phase times before interpreting.'
            break
        }
        if ($phase -eq 'armed' -and (($f8 -and $foreground) -or (Test-Path -LiteralPath (Join-Path $output 'baseline.command')))) {
            Take-GameSnapshot 'before-battles'
            Set-CapturePhase 'battles' 'Baseline saved. Battle a few times, then stop battling and press F9 in the game.'
        }
        elseif ($phase -eq 'battles' -and (($f9 -and $foreground) -or (Test-Path -LiteralPath (Join-Path $output 'after-battles.command')))) {
            Take-GameSnapshot 'after-battles'
            $cooldownStarted = [DateTime]::UtcNow
            Set-CapturePhase 'cooldown' 'Stop battling. Stay on the map; the last snapshot and recording stop are automatic.'
        }
        elseif ($phase -eq 'cooldown' -and ($now - $cooldownStarted).TotalSeconds -ge $CooldownSeconds) {
            Take-GameSnapshot 'after-cooldown'
            $complete = $true
            break
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $complete -and $phase -ne 'user_finished') { Set-CapturePhase 'timeout' 'Time limit reached; preserving partial capture.' }
}
catch { $failure = $_; Write-Warning $_ }
finally {
    if ($configured -and -not $game.HasExited) {
        try { Invoke-WprChecked @('-snapshotconfig', 'heap', '-pid', [string]$TargetProcessId, 'disable') }
        catch { $failure = $_; Write-Warning $_ }
    }
    if ($recording) {
        try {
            Set-CapturePhase 'saving' 'Saving this named WPR recording. Keep the game open.'
            Invoke-WprChecked @('-stop', (Join-Path $output 'heap-snapshots.etl'), '-instancename', $instance)
            $stopSucceeded = $true
        }
        catch { $failure = $_; Write-Warning $_ }
    }
    $outcome = if ($failure -or -not $stopSucceeded) { 'failed' } elseif ($complete) { 'captured' } else { 'partial' }
    @('outcome=' + $outcome; 'complete=' + $complete; 'trace_saved=' + $stopSucceeded;
      'ended_utc=' + [DateTime]::UtcNow.ToString('o'); 'error=' + [string]$failure) |
        Set-Content -LiteralPath (Join-Path $output 'result.txt') -Encoding UTF8
    Set-CapturePhase $outcome 'Capture helper finished. Leave the game open for analysis; only this helper session was stopped.'
    Write-Host "Output: $output"
    if ($NotifyOnFinish) {
        $message = if ($outcome -eq 'captured') {
            "All three heap snapshots have been saved. You may now close the game.`r`n`r`nOutput: $output"
        } else {
            "Capture ended before all three snapshots completed. Existing data was preserved.`r`n`r`nOutput: $output"
        }
        $null = [GameHeapCaptureKeys]::MessageBox([IntPtr]::Zero, $message, 'Ruffle heap capture: ' + $outcome, 0x00040040)
    }
}
if ($failure) { throw $failure }
if (-not $stopSucceeded) { throw 'No trace saved; inspect result.txt and the named session in capture-build.txt.' }
