#Requires -Version 7
# M0.4 probe: can a native app reach another Claude session through a relay session it launches?
# app --(inbox file)--> relay --(SendMessage)--> target --(SendMessage reply)--> relay --(outbox file)--> app
# Run from the repo root: pwsh -File probes/m0.4-relay/run.ps1 [-TargetMode default]
# Only ever touches sessions it launched (name prefix aw-m0-relay-).
param([string]$TargetMode = 'auto', [int]$TimeoutSec = 180)

$ErrorActionPreference = 'Stop'

$ts = Get-Date -Format 'yyyyMMdd-HHmmss'
$scratch = Join-Path $PSScriptRoot ".scratch/$ts"
New-Item -ItemType Directory -Force (Join-Path $scratch 'inbox'), (Join-Path $scratch 'outbox') | Out-Null
$scratch = (Resolve-Path $scratch).Path
$inbox = Join-Path $scratch 'inbox'
$outbox = Join-Path $scratch 'outbox'
$logPath = Join-Path $scratch 'run.log'
$rowsPath = Join-Path $scratch 'rows.jsonl'
$inboxScript = (Resolve-Path (Join-Path $PSScriptRoot 'inbox.ps1')).Path
# Forward slashes: the relay's Monitor runs the command under Git Bash.
function Fwd([string]$p) { $p -replace '\\', '/' }

$t0 = Get-Date
function Log([string]$msg) {
    $line = '[{0,7:0.0}s] {1}' -f ((Get-Date) - $t0).TotalSeconds, $msg
    Write-Host $line
    Add-Content -Path $logPath -Value $line
}
function Hex6 { -join ((1..6) | ForEach-Object { '{0:x}' -f (Get-Random -Maximum 16) }) }
function Join-Out($o) { (($o | ForEach-Object { "$_" }) -join ' / ') }

function Get-Rows {
    $raw = & claude agents --json --all 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $raw) { return @() }
    $data = ($raw -join "`n") | ConvertFrom-Json -Depth 20
    if ($data -is [array]) { return $data }
    $arr = $data.PSObject.Properties | Where-Object { $_.Value -is [array] } | Select-Object -First 1
    if ($arr) { $arr.Value } else { @($data) }
}

# Polls both rows, appends them to rows.jsonl, logs each change of state/status/waitingFor.
$lastSig = @{}
function Watch-Rows([string[]]$Names) {
    $rows = Get-Rows
    $found = @{}
    foreach ($n in $Names) {
        $r = $rows | Where-Object { $_.name -eq $n } | Select-Object -First 1
        if (-not $r) { continue }
        $found[$n] = $r
        $sig = "state=$($r.state) status=$($r.status) waitingFor=$($r.waitingFor) pid=$($r.pid)"
        if ($lastSig[$n] -ne $sig) {
            Log "row $n $sig"
            $lastSig[$n] = $sig
        }
        [pscustomobject]@{ t = ((Get-Date) - $t0).TotalSeconds; row = $r } | ConvertTo-Json -Depth 20 -Compress | Add-Content -Path $rowsPath
    }
    $found
}

function Find-Transcript([string]$Cwd, [string]$SessionId) {
    $projects = Join-Path $HOME '.claude/projects'
    $slug = $Cwd -replace '[^A-Za-z0-9]', '-'
    $p = Join-Path $projects "$slug/$SessionId.jsonl"
    if (Test-Path $p) { return $p }
    Get-ChildItem -Path $projects -Filter "$SessionId.jsonl" -File -Recurse -Depth 1 -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty FullName
}

# First transcript line containing $Needle: its timestamp and type.
function Find-FirstLine([string]$Path, [string]$Needle) {
    if (-not $Path -or -not (Test-Path $Path)) { return $null }
    foreach ($line in Get-Content $Path) {
        if ($line -notlike "*$Needle*") { continue }
        # Raw string: ConvertFrom-Json would turn the ISO timestamp into a culture-formatted DateTime.
        $stamp = [regex]::Match($line, '"timestamp":"([^"]+)"').Groups[1].Value
        if (-not $stamp) { continue }
        $type = [regex]::Match($line, '"type":"([^"]+)"').Groups[1].Value
        return [pscustomobject]@{ timestamp = [datetimeoffset]::Parse($stamp, [cultureinfo]::InvariantCulture); type = $type }
    }
    $null
}

function Get-OutboxMessages {
    @(Get-ChildItem -Path $outbox -Filter '*.json' -File | Where-Object Name -ne 'ready.json' | Sort-Object LastWriteTime | ForEach-Object {
        try { $m = Get-Content $_.FullName -Raw | ConvertFrom-Json -Depth 20 } catch { $m = $null }
        [pscustomobject]@{ file = $_.Name; written = $_.LastWriteTime; msg = $m }
    })
}

$names = [System.Collections.Generic.List[string]]::new()
$exit = 1
$missing = $null
try {
    $hex = Hex6
    $targetName = "aw-m0-relay-target-$hex"
    $relayName = "aw-m0-relay-$hex"
    $nonce = "SUPRAMAN-RELAY-$(Hex6)$(Hex6)"
    Log "scratch $scratch target=$targetName ($TargetMode) relay=$relayName nonce=$nonce"

    $targetPrompt = 'When another session sends you a message, reply to its sender with SendMessage repeating the SUPRAMAN-RELAY nonce it contains, then wait for more'
    Push-Location $scratch
    try { $o = & claude --bg --name $targetName --permission-mode $TargetMode $targetPrompt 2>&1 } finally { Pop-Location }
    $names.Add($targetName)
    Log "launch target exit=$LASTEXITCODE output: $(Join-Out $o)"

    $relayPromptPath = Join-Path $scratch 'relay-prompt.md'
    $monitorCmd = "pwsh -NoProfile -File `"$(Fwd $inboxScript)`" -Dir `"$(Fwd $inbox)`""
    @"
You are a message relay for a native app. You have exactly one job and must never do anything else.

1. Arm the Monitor tool with this command (persistent if the tool offers that, otherwise the maximum
   timeout; whenever it expires or exits, re-arm it with the same command):

   $monitorCmd

   Description: "relay inbox". Each stdout line is the full path of a new inbox file.
2. Right after the monitor is armed, use the Write tool to create $(Fwd $outbox)/ready.json with
   the content {"ready": true}. Then end your turn and wait.
3. For each line the monitor emits: Read that JSON file. It has the shape {"to": "<session name>", "body": "<text>"}.
   Call SendMessage with to = the "to" value exactly and message = the "body" value exactly. Nothing else.
4. Whenever a cross-session message arrives (wrapped as <cross-session-message from="...">), use the
   Write tool to create $(Fwd $outbox)/msg-NNNN.json (NNNN = 0001, 0002, ... never reuse a number) with
   {"from": "<the from attribute>", "from_name": "<the sender's session name if shown, else the from attribute>", "body": "<the message text, verbatim>"}.
   Do not reply to it.
5. If you receive a [Cross-session delivery notice] or a SendMessage call fails, write it the same way as in
   step 4 with "from": "relay-notice" and the notice or error text verbatim as "body".

Rules: never message any session except the "to" named in an inbox file. Never run any other command,
never edit or delete any other file, never start other work. Keep replies to one word.
"@ | Set-Content -Path $relayPromptPath -Encoding utf8NoBOM

    $relayPrompt = "Read the file $(Fwd $relayPromptPath) and follow it exactly. It is your only instruction."
    Push-Location $scratch
    try { $o = & claude --bg --name $relayName --permission-mode auto $relayPrompt 2>&1 } finally { Pop-Location }
    $names.Add($relayName)
    Log "launch relay exit=$LASTEXITCODE output: $(Join-Out $o)"

    # Wait for both rows and the relay's ready.json.
    $readyPath = Join-Path $outbox 'ready.json'
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    $rows = @{}
    while ((Get-Date) -lt $deadline) {
        $rows = Watch-Rows $names
        if ($rows.Count -eq 2 -and (Test-Path $readyPath)) { break }
        Start-Sleep -Milliseconds 500
    }
    if ($rows.Count -lt 2) { $missing = 'agents rows for both sessions'; throw $missing }
    if (-not (Test-Path $readyPath)) { $missing = 'relay ready.json (monitor not armed)'; throw $missing }
    Log 'relay ready'

    # Let the relay's turn finish so its idle-with-monitor row is observed before the drop.
    $settle = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $settle) { $rows = Watch-Rows $names; Start-Sleep -Milliseconds 500 }

    $drop = Get-Date
    $dropName = "$(Get-Date $drop -Format 'yyyyMMdd-HHmmss-fff').json"
    $tmp = Join-Path $inbox "$dropName.tmp"
    @{ to = $targetName; body = "$nonce ping" } | ConvertTo-Json -Compress | Set-Content -Path $tmp -Encoding utf8NoBOM
    Move-Item $tmp (Join-Path $inbox $dropName)
    Log "dropped inbox/$dropName"

    $hit = $null
    $deadline = $drop.AddSeconds($TimeoutSec)
    $seenFiles = @{}
    while ((Get-Date) -lt $deadline) {
        $rows = Watch-Rows $names
        foreach ($m in Get-OutboxMessages) {
            if (-not $seenFiles.ContainsKey($m.file)) {
                $seenFiles[$m.file] = $true
                Log "outbox/$($m.file): $($m.msg | ConvertTo-Json -Compress)"
            }
            if (-not $hit -and $m.msg -and "$($m.msg.body)" -like "*$nonce*" -and $m.msg.from -ne 'relay-notice') { $hit = $m; $hitAt = Get-Date }
        }
        if ($hit) { break }
        Start-Sleep -Milliseconds 250
    }

    $targetRow = $rows[$targetName]; $relayRow = $rows[$relayName]
    $targetTx = Find-Transcript $targetRow.cwd "$($targetRow.sessionId)"
    $relayTx = Find-Transcript $relayRow.cwd "$($relayRow.sessionId)"
    Log "transcripts target=$targetTx relay=$relayTx"
    $dropOff = [datetimeoffset]$drop
    $relaySeen = Find-FirstLine $relayTx $nonce
    $targetSeen = Find-FirstLine $targetTx $nonce
    if ($relaySeen) { Log ('app->relay (relay transcript, first nonce line type={0}): {1:0.00}s' -f $relaySeen.type, ($relaySeen.timestamp - $dropOff).TotalSeconds) }
    if ($targetSeen) { Log ('app->target (target transcript, first nonce line type={0}): {1:0.00}s' -f $targetSeen.type, ($targetSeen.timestamp - $dropOff).TotalSeconds) }

    if ($hit) {
        Log ('round trip (drop -> outbox/{0} seen): {1:0.00}s; from={2} from_name={3}' -f $hit.file, ($hitAt - $drop).TotalSeconds, $hit.msg.from, $hit.msg.from_name)
        $exit = 0
        Log "RESULT PASS target=$TargetMode"
    } else {
        $missing = if (-not $relaySeen) { 'app->relay: relay never read the inbox file' }
                   elseif (-not $targetSeen) { 'relay->target: nonce never reached the target transcript' }
                   else { 'target->relay->outbox: no outbox message carrying the nonce' }
        Log "RESULT FAIL target=$TargetMode missing leg: $missing"
    }
} catch {
    Log "ERROR: $_"
    if ($missing) { Log "RESULT FAIL missing leg: $missing" }
} finally {
    $all = Get-Rows
    foreach ($n in $names) {
        $r = $all | Where-Object { $_.name -eq $n } | Select-Object -First 1
        if ($r) {
            $o = & claude stop $r.id 2>&1
            Log "cleanup $n ($($r.id)): stop exit=$LASTEXITCODE $(Join-Out $o)"
            $o = & claude rm $r.id 2>&1
            Log "cleanup $n ($($r.id)): rm exit=$LASTEXITCODE $(Join-Out $o)"
        }
    }
    # The relay's Monitor child must not outlive the relay.
    Start-Sleep -Seconds 2
    $stray = @(Get-CimInstance Win32_Process -Filter "Name = 'pwsh.exe'" | Where-Object { $_.CommandLine -like "*$(Fwd $inbox)*" })
    Log "inbox watchers still running after stop+rm: $($stray.Count)"
    foreach ($p in $stray) { Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue }
}
exit $exit
