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

.EXAMPLE
.\scripts\recorder-api-test.ps1 -Token 0123abcd...
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$Token,

    [int]$Port = 4466
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
            $received += $message
        }
    }
    return , $received
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

Write-Host "== Good token, port $Port =="
$connection = Connect-Recorder $Port
Send-Message $connection @{ op = 'auth'; token = $Token }
[void](Receive-For $connection 1000)

[void](Invoke-Command-Api $connection @{ op = 'getStatus'; id = 1 })
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
