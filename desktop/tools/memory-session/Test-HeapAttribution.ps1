[CmdletBinding()]
param([switch]$ValidateOnly)

$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..\..'))
$output = Join-Path $repoRoot ('target\heap-attribution\' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
New-Item -ItemType Directory -Path $output | Out-Null
$exe = Join-Path $output 'ruffle_heap_fixture.exe'
$pdb = Join-Path $output 'ruffle_heap_fixture.pdb'
$source = Join-Path $PSScriptRoot 'heap_fixture.rs'
Copy-Item -LiteralPath $source -Destination (Join-Path $output 'heap_fixture.rs')
& rustc $source --edition=2024 --crate-name ruffle_heap_fixture -C debuginfo=2 -C opt-level=1 -o $exe
if ($LASTEXITCODE -ne 0) { throw 'Offline fixture compilation failed.' }
if (-not (Test-Path -LiteralPath $pdb)) { throw 'Expected matching PDB was not generated.' }

# Always prove the local workload and Rust symbol lookup before recording.
$smoke = Join-Path $output 'smoke'
& $exe $smoke --self-test
if ($LASTEXITCODE -ne 0) { throw 'Offline allocation/release self-test failed.' }
$probe = Get-Content -LiteralPath (Join-Path $smoke 'symbol-probe.txt') -Raw
if ($probe -notmatch 'symbol_probe_site' -or $probe -notmatch 'heap_fixture.rs') {
    throw 'Symbol self-test did not resolve the fixture function and source file.'
}
@(
    'fixture_payload_baseline_bytes=0'
    'fixture_payload_allocated_bytes=18874368'
    'fixture_payload_after_release_bytes=2097152'
    'expected_transient_release_bytes=16777216'
    'block_count_retained=8'
    'block_count_transient=64'
    'block_bytes=262144'
    'scope=offline fixture only; expected payload excludes runtime and allocator overhead'
    'symbol_probe=passed (Rust backtrace; WPA stack resolution remains to be verified)'
) | Set-Content -LiteralPath (Join-Path $output 'expected.txt') -Encoding UTF8
foreach ($artifact in @($exe, $pdb, (Join-Path $output 'heap_fixture.rs'))) {
    $algorithm = [Security.Cryptography.SHA256]::Create()
    $stream = [IO.File]::OpenRead($artifact)
    try { $hash = [BitConverter]::ToString($algorithm.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
    finally { $stream.Dispose(); $algorithm.Dispose() }
    ([IO.Path]::GetFileName($artifact) + ' sha256=' + $hash) |
        Add-Content -LiteralPath (Join-Path $output 'artifacts.txt') -Encoding UTF8
}

if ($ValidateOnly) {
    'not_started: ValidateOnly; no ETW recording or process tracing configuration changed' |
        Set-Content -LiteralPath (Join-Path $output 'capture-status.txt') -Encoding UTF8
    Write-Host "Offline allocation and symbol self-tests passed. Output: $output"
    return
}

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    'not_started: Windows requires administrator rights for WPR HeapSnapshot' |
        Set-Content -LiteralPath (Join-Path $output 'capture-status.txt') -Encoding UTF8
    throw "WPR HeapSnapshot needs an administrator PowerShell. Offline self-tests passed: $output"
}

function Invoke-WprChecked([string[]]$Arguments) {
    $result = & "$env:WINDIR\System32\wpr.exe" @Arguments 2>&1
    $code = $LASTEXITCODE
    $result | Add-Content -LiteralPath (Join-Path $output 'wpr.log') -Encoding UTF8
    if ($code -ne 0) { throw "WPR failed ($code): $($Arguments -join ' ')" }
}
function Wait-FixtureMarker([string]$Name) {
    $deadline = [DateTime]::UtcNow.AddSeconds(25)
    while (-not (Test-Path -LiteralPath (Join-Path $workload $Name))) {
        if ($fixture.HasExited) { throw "Fixture exited before $Name" }
        if ([DateTime]::UtcNow -ge $deadline) { throw "Timed out waiting for $Name" }
        Start-Sleep -Milliseconds 100
    }
}
function Send-FixtureCommand([string]$Name) {
    [IO.File]::WriteAllText((Join-Path $workload $Name), 'go')
}
function Take-FixtureSnapshot([string]$Stage, [long]$ExpectedBytes) {
    $before = [DateTime]::UtcNow.ToString('o')
    Invoke-WprChecked @('-singlesnapshot', 'heap', [string]$fixture.Id, '-instancename', $instance)
    ($Stage + "`t" + $fixture.Id + "`t" + $before + "`t" + [DateTime]::UtcNow.ToString('o') + "`t" + $ExpectedBytes) |
        Add-Content -LiteralPath (Join-Path $output 'snapshots.tsv') -Encoding UTF8
}

$instance = 'RuffleHeap-' + [Guid]::NewGuid().ToString('N')
$workload = Join-Path $output 'workload'
New-Item -ItemType Directory -Path $workload | Out-Null
$fixture = $null
$recording = $false
$configured = $false
$completed = $false
$stopSucceeded = $false
try {
    # Unique instance: never stop/cancel a pre-existing WPR recording.
    Invoke-WprChecked @('-start', 'HeapSnapshot', '-filemode', '-instancename', $instance)
    $recording = $true
    $fixture = Start-Process -FilePath $exe -ArgumentList ('"' + $workload + '"') -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $output 'fixture-stdout.txt') `
        -RedirectStandardError (Join-Path $output 'fixture-stderr.txt')
    Wait-FixtureMarker 'baseline.ready'
    # PID configuration expires with this fixture; no persistent IFEO edits.
    Invoke-WprChecked @('-snapshotconfig', 'heap', '-pid', [string]$fixture.Id, 'enable')
    $configured = $true
    Take-FixtureSnapshot 'baseline' 0
    Send-FixtureCommand 'allocate.command'
    Wait-FixtureMarker 'allocated.ready'
    Take-FixtureSnapshot 'allocated' 18874368
    Send-FixtureCommand 'release.command'
    Wait-FixtureMarker 'released.ready'
    Take-FixtureSnapshot 'released' 2097152
    $completed = $true
}
finally {
    if ($configured -and $fixture -and -not $fixture.HasExited) {
        try { Invoke-WprChecked @('-snapshotconfig', 'heap', '-pid', [string]$fixture.Id, 'disable') }
        catch { Write-Warning $_ }
    }
    if ($recording) {
        $trace = Join-Path $output 'heap-snapshots.etl'
        try {
            Invoke-WprChecked @('-stop', $trace, '-instancename', $instance)
            $stopSucceeded = $true
            $(if ($completed) { 'captured: analysis of allocation stacks still required' } else { 'partial: capture interrupted; inspect wpr.log' }) |
                Set-Content -LiteralPath (Join-Path $output 'capture-status.txt') -Encoding UTF8
        }
        catch {
            'stop_failed: inspect wpr.log; named session=' + $instance |
                Set-Content -LiteralPath (Join-Path $output 'capture-status.txt') -Encoding UTF8
            Write-Warning $_
        }
    }
    if ($fixture -and -not $fixture.HasExited) {
        # Exit all waits, including those before allocation if capture failed.
        'allocate.command', 'release.command', 'exit.command' | ForEach-Object { Send-FixtureCommand $_ }
        if (-not $fixture.WaitForExit(10000)) { $fixture.Kill() }
    }
    Write-Host "Output: $output"
}
if (-not $stopSucceeded -or -not (Test-Path -LiteralPath (Join-Path $output 'heap-snapshots.etl'))) {
    throw 'No ETL produced. Inspect capture-status.txt and wpr.log.'
}
