[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateRange(1,2147483647)][int]$RootProcessId,
    [ValidateRange(1,180)][int]$Seconds = 60
)
$ErrorActionPreference = 'Stop'
$taskRoot = Get-Process -Id $RootProcessId
if ($taskRoot.ProcessName -notin @('omniterm-desktop','omniterm-test-host')) { throw 'Expected the caller-owned Omniterm root.' }
$taskRootStart = $taskRoot.StartTime.ToUniversalTime().Ticks
$taskLogical = [Environment]::ProcessorCount
$taskClock = [Diagnostics.Stopwatch]::StartNew()
$taskPrevious = @{}
$taskVersions = @{}
$taskLastMs = 0.0
$taskFirstMs = 0.0
$taskSamples = [Collections.Generic.List[object]]::new()
for ($taskSample = 0; $taskSample -le $Seconds; $taskSample++) {
    $taskRoot.Refresh()
    if ($taskRoot.HasExited -or $taskRoot.StartTime.ToUniversalTime().Ticks -ne $taskRootStart) { throw 'Owned root exited during sampling.' }
    $taskInventory = @(Get-CimInstance Win32_Process -Property ProcessId,ParentProcessId,CreationDate,Name)
    $taskIds = [Collections.Generic.HashSet[int]]::new()
    [void]$taskIds.Add($RootProcessId)
    do {
        $taskAdded = $false
        foreach ($taskRow in $taskInventory) {
            if ($taskIds.Contains([int]$taskRow.ParentProcessId) -and $taskRow.CreationDate.ToUniversalTime().Ticks -ge $taskRootStart) {
                if ($taskIds.Add([int]$taskRow.ProcessId)) { $taskAdded = $true }
            }
        }
    } while ($taskAdded)
    $taskNowMs = $taskClock.Elapsed.TotalMilliseconds
    if ($taskSample -eq 0) { $taskFirstMs = $taskNowMs }
    $taskProcesses = @()
    $taskCpuDelta = 0.0
    foreach ($taskOwnedId in $taskIds) {
        try {
            $taskProcess = Get-Process -Id $taskOwnedId -ErrorAction Stop
            $taskStart = $taskProcess.StartTime.ToUniversalTime().Ticks
            if ($taskStart -lt $taskRootStart) { continue }
            $taskIdentity = "$taskOwnedId/$taskStart"
            $taskCpu = $taskProcess.TotalProcessorTime.TotalSeconds
            if ($taskPrevious.ContainsKey($taskIdentity)) { $taskCpuDelta += [Math]::Max(0.0, $taskCpu - $taskPrevious[$taskIdentity]) }
            $taskPrevious[$taskIdentity] = $taskCpu
            if (!$taskVersions.ContainsKey($taskIdentity)) {
                $taskVersions[$taskIdentity] = try { $taskProcess.MainModule.FileVersionInfo.FileVersion } catch { 'unavailable' }
            }
            $taskProcesses += [ordered]@{id=$taskOwnedId;start_ticks="$taskStart";name=$taskProcess.ProcessName;file_version=$taskVersions[$taskIdentity];cpu_seconds=$taskCpu;private_bytes=$taskProcess.PrivateMemorySize64;working_set_bytes=$taskProcess.WorkingSet64}
        } catch { $taskProcesses += [ordered]@{id=$taskOwnedId;unavailable=$true} }
    }
    $taskDeltaMs = $taskNowMs - $taskLastMs
    $taskCpuPercent = if ($taskSample -gt 0 -and $taskDeltaMs -gt 0) {100000.0 * $taskCpuDelta / $taskDeltaMs} else {0.0}
    $taskPrivate = [long]0
    $taskWorking = [long]0
    foreach ($taskRecord in $taskProcesses) {
        if ($taskRecord.Contains('private_bytes')) { $taskPrivate += $taskRecord.private_bytes; $taskWorking += $taskRecord.working_set_bytes }
    }
    if ($taskPrivate -le 0 -or $taskWorking -le 0) { throw 'No valid process memory counters were captured.' }
    $taskSamples.Add([ordered]@{elapsed_ms=$taskNowMs;cpu_one_core_percent=$taskCpuPercent;cpu_machine_percent=$taskCpuPercent/$taskLogical;private_bytes=$taskPrivate;working_set_sum_bytes=$taskWorking;processes=$taskProcesses})
    $taskLastMs = $taskNowMs
    if ($taskSample -lt $Seconds) {
        $taskDelayMs = [Math]::Max(1,$taskFirstMs+($taskSample+1)*1000 - $taskClock.Elapsed.TotalMilliseconds)
        Start-Sleep -Milliseconds ([int]$taskDelayMs)
    }
}
[ordered]@{root_id=$RootProcessId;root_start_ticks="$taskRootStart";logical_cpus=$taskLogical;powershell_version=$PSVersionTable.PSVersion.ToString();duration_ms=$taskClock.Elapsed.TotalMilliseconds;method='owned descendant snapshots; private allocated bytes and working-set sum (shared pages may duplicate); short-lived child CPU before first observation excluded';samples=$taskSamples} | ConvertTo-Json -Depth 7 -Compress
