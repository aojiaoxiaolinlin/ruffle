[CmdletBinding()]
param(
    [ValidateSet('observe', 'record', 'replay')][string]$Mode = 'observe',
    [int]$ProcessId = 0,
    [string]$Recording = '',
    [string]$OutputDirectory = '',
    [switch]$Unattended,
    [switch]$ValidateOnly,
    [switch]$SmokeTest
)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSEdition -ne 'Desktop' -or -not [Environment]::Is64BitProcess) {
    throw 'Use 64-bit Windows PowerShell 5.1 (powershell.exe), not PowerShell 7 (pwsh.exe).'
}
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -Path (Join-Path $PSScriptRoot 'MemorySession.cs') -ReferencedAssemblies @(
    'System.Windows.Forms', 'System.Drawing', 'System.Runtime.Serialization', 'System.Xml'
)
if ($ValidateOnly) {
    [MemorySession.SelfTest]::Run()
    Write-Host 'Compilation and self-tests passed. No input hooks or replay started.'
    return
}
if ($SmokeTest) {
    if (-not $OutputDirectory -or (Test-Path $OutputDirectory)) {
        throw 'SmokeTest requires a new -OutputDirectory.'
    }
    [MemorySession.SelfTest]::ReplaySmoke([IO.Path]::GetFullPath($OutputDirectory))
    return
}
if ($Mode -eq 'replay' -and -not $Recording) {
    throw 'Replay requires -Recording <recording.json>.'
}
if ($ProcessId -eq 0) {
    $brandPattern = '*' + [char]0x6e38 + [char]0x620f + [char]0x76d2 + '*'
    $candidates = @(Get-Process | Where-Object {
        $_.MainWindowHandle -ne 0 -and ($_.ProcessName -like '*ruffle*' -or $_.ProcessName -like $brandPattern)
    })
    if ($candidates.Count -ne 1) {
        throw 'Specify the game PID using -ProcessId (Task Manager > Details).'
    }
    $ProcessId = $candidates[0].Id
}
if (-not $OutputDirectory) {
    $OutputDirectory = Join-Path (Join-Path $PSScriptRoot 'sessions') ('memory-' + $Mode + '-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
}
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path $OutputDirectory) { throw 'OutputDirectory must be a new directory.' }
if ($Recording) { $Recording = (Resolve-Path $Recording).Path }
New-Item -ItemType Directory -Path $OutputDirectory | Out-Null
Copy-Item (Join-Path $PSScriptRoot 'notes-template.md') (Join-Path $OutputDirectory 'notes.md')
[MemorySession.Session]::Run($ProcessId, $Mode, $Recording, $OutputDirectory, $Unattended.IsPresent)
