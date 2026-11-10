[CmdletBinding()]
param([Parameter(Mandatory)][string]$Binary,[Parameter(Mandatory)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$taskOutput = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $taskOutput) { throw 'Use a fresh owned output directory.' }
[void](New-Item -ItemType Directory -Path $taskOutput)
$taskSamples = @()
$taskCurrent = $null
try {
    foreach ($taskCase in @('fresh-1','fresh-2','fresh-3','warm-1','warm-2','warm-3')) {
        $taskProfile = if ($taskCase.StartsWith('fresh')) {$taskCase} else {'fresh-3'}
        $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $taskOutput $taskProfile
        $taskClock = [Diagnostics.Stopwatch]::StartNew()
        $taskCurrent = Start-Process -FilePath ([IO.Path]::GetFullPath($Binary)) -WorkingDirectory $taskOutput -WindowStyle Hidden -PassThru
        do {
            $taskCurrent.Refresh()
            if ($taskCurrent.HasExited) { throw 'Owned ordinary app exited before its window was ready.' }
            if ($taskClock.Elapsed.TotalSeconds -gt 15) { throw 'Window-ready proxy timed out.' }
            Start-Sleep -Milliseconds 20
        } until ($taskCurrent.MainWindowHandle -ne 0 -and $taskCurrent.MainWindowTitle -eq 'Omniterm' -and $taskCurrent.Responding)
        $taskSamples += [ordered]@{case=$taskCase;profile=$taskProfile;window_ready_proxy_ms=$taskClock.Elapsed.TotalMilliseconds;id=$taskCurrent.Id}
        if ($taskCase -eq 'warm-3') {
            Start-Sleep -Seconds 2
            $taskCounters = & (Join-Path $PSScriptRoot 'resource-probe.ps1') -RootProcessId $taskCurrent.Id -Seconds 60
            [IO.File]::WriteAllText((Join-Path $taskOutput 'idle.json'),$taskCounters,[Text.UTF8Encoding]::new($false))
        }
        if (!$taskCurrent.CloseMainWindow() -or !$taskCurrent.WaitForExit(5000)) { throw 'Owned empty ordinary app did not close cleanly.' }
        $taskCurrent = $null
        Start-Sleep -Milliseconds 500
    }
    $taskResult = [ordered]@{result='PASS';binary_sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $Binary).Hash.ToLower();startup=$taskSamples;startup_metric='process launch to native window handle/title/responding; not rendered content or OS-cold boot';user_profile='process-local test-owned profile only'}
    $taskJson = $taskResult | ConvertTo-Json -Depth 5
    [IO.File]::WriteAllText((Join-Path $taskOutput 'result.json'),$taskJson,[Text.UTF8Encoding]::new($false))
    Write-Output 'PASS: production startup proxies and 60-second idle tree counters'
} finally {
    if ($null -ne $taskCurrent -and !$taskCurrent.HasExited) {
        # This handle was returned by our Start-Process; never find/stop by name or an old PID.
        [void]$taskCurrent.CloseMainWindow()
        if (!$taskCurrent.WaitForExit(5000)) { $taskCurrent.Kill(); $taskCurrent.WaitForExit() }
    }
}
