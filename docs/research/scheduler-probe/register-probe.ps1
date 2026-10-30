# sched/1 probes (a)-(d) from a real scheduled task. Run by René, see docs/research/scheduler.md, "Probes".
# Touches only \ForemanProbe-sched-1\ and <worktree>\.probe-run; prints no token; never sleeps the PC.
param(
    [string]$Exe = 'C:\fmbuild\sched-1\probe-target\release\scheduler-probe.exe',
    [string]$Repo = 'P:\Projects\LarchSys\AutoWork'
)
$ErrorActionPreference = 'Stop'
$worktree = (Resolve-Path "$PSScriptRoot\..\..\..").Path
$root = Join-Path $worktree '.probe-run'
$folder = '\ForemanProbe-sched-1\'
$out = Join-Path $root 'register-probe-output.txt'

if (Test-Path $root) { Remove-Item -Recurse -Force $root }
New-Item -ItemType Directory -Force "$root\home", "$root\scratch", "$root\bin" | Out-Null
Copy-Item $Exe "$root\bin\scheduler-probe.exe"
Start-Transcript -Path $out | Out-Null
try {
    "== elevated shell: $(([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))"
    "== owner of ${Repo}: $((Get-Acl $Repo).Owner); of .git: $((Get-Acl "$Repo\.git").Owner)"

    # (b) setup, interactive: a fake WorkState on 127.0.0.1 signs in through the DPAPI FileStore of the scratch home.
    $serve = Start-Process -FilePath "$root\bin\scheduler-probe.exe" -ArgumentList 'serve', "`"$root\home`"" -PassThru -WindowStyle Hidden
    for ($i = 0; -not (Test-Path "$root\home\ready") -and $i -lt 60; $i++) { Start-Sleep -Milliseconds 500 }
    "== serve (interactive): $(Get-Content "$root\home\ready" -ErrorAction SilentlyContinue)"

    $user = "$env:USERDOMAIN\$env:USERNAME"
    $start = (Get-Date).AddHours(1).ToString('yyyy-MM-ddTHH:mm:ss')
    $xml = @"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>sched/1 probe, removed by register-probe.ps1</Description></RegistrationInfo>
  <Triggers><TimeTrigger><StartBoundary>$start</StartBoundary><Enabled>true</Enabled></TimeTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>$user</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <WakeToRun>true</WakeToRun>
    <ExecutionTimeLimit>PT10M</ExecutionTimeLimit>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author"><Exec><Command>$root\bin\scheduler-probe.exe</Command><Arguments>task "$root\home" "$root\scratch" "$Repo"</Arguments><WorkingDirectory>$root</WorkingDirectory></Exec></Actions>
</Task>
"@
    Register-ScheduledTask -TaskPath $folder -TaskName 'probe' -Xml $xml | Out-Null
    $task = Get-ScheduledTask -TaskPath $folder -TaskName 'probe'
    "== (d) registered, read back: LogonType=$($task.Principal.LogonType) RunLevel=$($task.Principal.RunLevel) WakeToRun=$($task.Settings.WakeToRun) StartWhenAvailable=$($task.Settings.StartWhenAvailable) trigger=$start"
    "== (d) powercfg /waketimers"
    powercfg /waketimers
    "== (d) powercfg /query SCHEME_CURRENT SUB_SLEEP RTCWAKE (Allow wake timers: 0 Disable, 1 Enable, 2 Important only)"
    powercfg /query SCHEME_CURRENT SUB_SLEEP RTCWAKE | Select-String 'Scheme GUID|Current AC|Current DC'

    "== (a)(b)(c) Start-ScheduledTask"
    Start-ScheduledTask -TaskPath $folder -TaskName 'probe'
    for ($i = 0; -not (Get-ChildItem "$root\home\task-*.log" -ErrorAction SilentlyContinue) -and $i -lt 180; $i++) { Start-Sleep 1 }
    for ($i = 0; (Get-ScheduledTask -TaskPath $folder -TaskName 'probe').State -eq 'Running' -and $i -lt 30; $i++) { Start-Sleep 1 }
    $info = Get-ScheduledTaskInfo -TaskPath $folder -TaskName 'probe'
    "== task result: LastRunTime=$($info.LastRunTime) LastTaskResult=$($info.LastTaskResult)"
    Get-ChildItem "$root\home\task-*.log" -ErrorAction SilentlyContinue | ForEach-Object { Get-Content $_.FullName }
} finally {
    "== cleanup"
    New-Item -ItemType File "$root\home\stop" -Force | Out-Null
    if (Get-ScheduledTask -TaskPath $folder -ErrorAction SilentlyContinue) {
        Unregister-ScheduledTask -TaskPath $folder -TaskName 'probe' -Confirm:$false
    }
    # Unregister leaves the folder; only the COM service deletes one.
    $svc = New-Object -ComObject Schedule.Service
    $svc.Connect()
    try { $svc.GetFolder('\').DeleteFolder('ForemanProbe-sched-1', 0) } catch { "DeleteFolder: $($_.Exception.Message)" }
    "task under ${folder}: $(@(Get-ScheduledTask -TaskPath $folder -ErrorAction SilentlyContinue).Count)"
    try { $svc.GetFolder($folder.TrimEnd('\')) | Out-Null; 'folder: STILL THERE' } catch { 'folder: gone' }
    Stop-Transcript | Out-Null
}
