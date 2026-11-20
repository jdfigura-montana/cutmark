<#
.SYNOPSIS
Exercises the Cutmark Recorder's local control API over a WebSocket.

.DESCRIPTION
Connects, authenticates, asks for the status, starts a take, adds a mark, pauses, resumes and stops,
printing every reply and event with a timestamp. Then it connects again with a wrong token and prints the error.
The Recorder must be running with the Control API switched on. Works in Windows PowerShell 5.1.

.PARAMETER Token
The API token, shown in the Recorder's Control API section.

.PARAMETER Port
The API port. Defaults to 4466.

.PARAMETER Prompter
Runs a fake Prompter instead of the basic run: authenticates with client "prompter", sends a 20-line script snapshot,
starts a take, prints the countdown events, waits for the take to begin, sends a scriptPosition every 500 ms for
lines 1 to 20, marks a Retake at line 8, then stops. The take folder gets _script.md and _script.csv.
With the Recorder's countdown on, the Recorder's own overlay stays hidden while this fake Prompter is connected.

.PARAMETER Countdown
Adds a countdown check to the basic run: sends start, waits 1 s, then sends stop during the countdown, and prints
countdownCancelled. Set the Recorder's countdown to 3 or 5 seconds first.

.EXAMPLE
.\scripts\recorder-api-test.ps1 -Token 0123abcd...

.EXAMPLE
.\scripts\recorder-api-test.ps1 -Token 0123abcd... -Countdown

.EXAMPLE
.\scripts\recorder-api-test.ps1 -Token 0123abcd... -Prompter
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$Token,

    [int]$Port = 4466,

    [switch]$Prompter,

    [switch]$Countdown
)

$ErrorActionPreference = 'Stop'

function Write-Stamped {
    param([string]$Tag, [string]$Text)
    Write-Host ('{0} {1,-5} {2}' -f (Get-Date -Format 'HH:mm:ss.fff'), $Tag, $Text)
}

function Connect-Recorder {
    param([int]$Port)
    $socket = New-Object System.Net.WebSockets.ClientWebSocket
    $uri = New-Object System.Uri("ws://127.0.0.1:$Port/")
    $socket.ConnectAsync($uri, [System.Threading.CancellationToken]::None).Wait()
    return [pscustomobject]@{
        Socket  = $socket
        Buffer  = New-Object byte[] 65536
        Text    = New-Object System.Text.StringBuilder
        Pending = $null
        Closed  = $false
        Seen    = @{}
    }
}

function Send-Message {
    param($Connection, $Message)
    $json = ConvertTo-Json -InputObject $Message -Compress -Depth 5
    Write-Stamped 'send' $json
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
    $segment = New-Object System.ArraySegment[byte] -ArgumentList @(, $bytes)
    $Connection.Socket.SendAsync($segment, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, [System.Threading.CancellationToken]::None).Wait()
}

# Prints every message that arrives within the given time, and returns them as parsed objects.
# Countdown events get an extra line, and every event name is remembered in $Connection.Seen.
function Receive-For {
    param($Connection, [int]$Milliseconds)
    $deadline = [DateTime]::UtcNow.AddMilliseconds($Milliseconds)
    $received = @()
    while (-not $Connection.Closed) {
        $left = [int]($deadline - [DateTime]::UtcNow).TotalMilliseconds
        if ($left -le 0) { break }
        try {
            if ($null -eq $Connection.Pending) {
                $segment = New-Object System.ArraySegment[byte] -ArgumentList @(, $Connection.Buffer)
                $Connection.Pending = $Connection.Socket.ReceiveAsync($segment, [System.Threading.CancellationToken]::None)
            }
            if (-not $Connection.Pending.Wait($left)) { break }
            $result = $Connection.Pending.Result
        }
        catch {
            Write-Stamped 'close' 'the connection ended'
            $Connection.Closed = $true
            $Connection.Pending = $null
            break
        }
        $Connection.Pending = $null
        if ($result.MessageType -eq [System.Net.WebSockets.WebSocketMessageType]::Close) {
            Write-Stamped 'close' 'the server closed the connection'
            $Connection.Closed = $true
            break
        }
        [void]$Connection.Text.Append([System.Text.Encoding]::UTF8.GetString($Connection.Buffer, 0, $result.Count))
        if ($result.EndOfMessage) {
            $text = $Connection.Text.ToString()
            [void]$Connection.Text.Clear()
            $message = $text | ConvertFrom-Json
            $tag = if ($message.op -eq 'event') { 'event' } else { 'reply' }
            Write-Stamped $tag $text
            if ($message.op -eq 'event') {
                $Connection.Seen[[string]$message.event] = $true
                if ($message.event -like 'countdown*') {
                    Write-Stamped 'count' "$($message.event) $($message.data | ConvertTo-Json -Compress)"
                }
            }
            $received += $message
        }
    }
    return , $received
}

# Waits up to the given time for an event with this name, which may already have arrived; true when it did.
function Wait-ForEvent {
    param($Connection, [string]$Name, [int]$Milliseconds)
    $deadline = [DateTime]::UtcNow.AddMilliseconds($Milliseconds)
    while (-not $Connection.Closed -and -not $Connection.Seen[$Name] -and [DateTime]::UtcNow -lt $deadline) {
        [void](Receive-For $Connection 250)
    }
    return [bool]$Connection.Seen[$Name]
}

# Sends a command and waits up to five seconds for the result that carries its id, printing events on the way.
function Invoke-Command-Api {
    param($Connection, $Message)
    Send-Message $Connection $Message
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not $Connection.Closed -and [DateTime]::UtcNow -lt $deadline) {
        foreach ($message in (Receive-For $Connection 250)) {
            if ($message.op -eq 'result' -and $message.id -eq $Message.id) { return $message }
        }
    }
    Write-Stamped 'warn' "no result arrived for id $($Message.id)"
}

function Close-Recorder {
    param($Connection)
    try {
        if (-not $Connection.Closed) {
            $Connection.Socket.CloseAsync([System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure, 'done', [System.Threading.CancellationToken]::None).Wait(2000) | Out-Null
        }
    }
    catch {
        # The server may have closed first.
    }
    $Connection.Socket.Dispose()
}

# Plays a Prompter that reads a 20-line script: a snapshot, a start, a position every 500 ms, a flub at line 8, then a stop.
function Invoke-FakePrompter {
    param([int]$Port, [string]$Token)
    $lines = 1..20 | ForEach-Object { "Line $_ of the fake script: this is the sentence the presenter reads aloud." }
    $markdown = "# Fake script`n`n" + ($lines -join "`n`n")

    Write-Host "== Fake Prompter, port $Port =="
    $connection = Connect-Recorder $Port
    Send-Message $connection @{ op = 'auth'; token = $Token; client = 'prompter' }
    [void](Receive-For $connection 1000)

    [void](Invoke-Command-Api $connection @{ op = 'scriptSnapshot'; id = 1; scriptId = 'fake-1'; title = 'Fake script'; markdown = $markdown })
    $connection.Seen.Remove('recordingStarted')
    [void](Invoke-Command-Api $connection @{ op = 'start'; id = 2 })
    # With a countdown on, the take begins only after it, and positions sent before that are ignored.
    if (-not (Wait-ForEvent $connection 'recordingStarted' 10000)) {
        Write-Stamped 'warn' 'no recordingStarted event arrived'
    }
    [void](Receive-For $connection 1500)

    for ($number = 1; $number -le 20; $number++) {
        $text = $lines[$number - 1]
        $token = ($text -split ' ')[0..1] -join ' '
        Send-Message $connection @{ op = 'scriptPosition'; scriptId = 'fake-1'; title = 'Fake script'; token = $token; line = $number; text = $text }
        if ($number -eq 8) {
            # A flub: the Prompter marks a Retake with the line it was on.
            [void](Invoke-Command-Api $connection @{ op = 'mark'; id = 3; type = 'Retake'; label = $text })
        }
        [void](Receive-For $connection 500)
    }

    [void](Invoke-Command-Api $connection @{ op = 'stop'; id = 4 })
    [void](Receive-For $connection 1500)
    Close-Recorder $connection
}

if ($Prompter) {
    Invoke-FakePrompter $Port $Token
    return
}

Write-Host "== Good token, port $Port =="
$connection = Connect-Recorder $Port
Send-Message $connection @{ op = 'auth'; token = $Token }
[void](Receive-For $connection 1000)

[void](Invoke-Command-Api $connection @{ op = 'getStatus'; id = 1 })

if ($Countdown) {
    Write-Host '== Countdown: start, then stop during the countdown =='
    $connection.Seen.Remove('countdownCancelled')
    [void](Invoke-Command-Api $connection @{ op = 'start'; id = 10 })
    [void](Receive-For $connection 1000)
    [void](Invoke-Command-Api $connection @{ op = 'stop'; id = 11 })
    [void](Receive-For $connection 1500)
    if ($connection.Seen['countdownCancelled']) {
        Write-Stamped 'ok' 'countdownCancelled arrived'
    }
    else {
        Write-Stamped 'warn' 'no countdownCancelled event arrived; is the Recorder countdown set to 3 or 5 seconds?'
    }
    Write-Host '== Basic run =='
}

[void](Invoke-Command-Api $connection @{ op = 'start'; id = 2 })
[void](Receive-For $connection 5000)
[void](Invoke-Command-Api $connection @{ op = 'mark'; id = 3; type = 'Note'; label = 'api test' })
[void](Invoke-Command-Api $connection @{ op = 'pause'; id = 4 })
[void](Receive-For $connection 3000)
[void](Invoke-Command-Api $connection @{ op = 'resume'; id = 5 })
[void](Receive-For $connection 5000)
[void](Invoke-Command-Api $connection @{ op = 'stop'; id = 6 })
[void](Receive-For $connection 1500)
Close-Recorder $connection

Write-Host '== Wrong token =='
$bad = Connect-Recorder $Port
Send-Message $bad @{ op = 'auth'; token = ('0' * 64) }
[void](Receive-For $bad 2000)
Close-Recorder $bad
