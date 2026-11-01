param(
    [string]$SavePath = 'C:\Users\Oncilla\Desktop\capricorn-main\_panel.png',
    [int]$Port = 9222
)

# Capture the panel page through CDP. Works even when the desktop is locked:
# the WebView2 engine paints the page, not the screen.

$ErrorActionPreference = 'Stop'

$targets = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json" -TimeoutSec 8
$page = $targets | Where-Object { $_.type -eq 'page' } | Select-Object -First 1
if (-not $page) { throw 'no page on port' }

$ws = New-Object System.Net.WebSockets.ClientWebSocket
$ct = [System.Threading.CancellationToken]::None
$ws.ConnectAsync([Uri]$page.webSocketDebuggerUrl, $ct).Wait(8000) | Out-Null

function Send-Cdp($id, $method, $params) {
    $payload = @{ id = $id; method = $method; params = $params } | ConvertTo-Json -Depth 6 -Compress
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($payload)
    $seg = New-Object System.ArraySegment[byte] -ArgumentList @(,$bytes)
    $ws.SendAsync($seg, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct).Wait(8000) | Out-Null
}

function Read-Cdp($wantId, $timeoutSec = 25) {
    $buffer = New-Object byte[] 1048576
    $deadline = (Get-Date).AddSeconds($timeoutSec)
    $text = ''
    while ((Get-Date) -lt $deadline) {
        $seg = New-Object System.ArraySegment[byte] -ArgumentList @(,$buffer)
        $task = $ws.ReceiveAsync($seg, $ct)
        if (-not $task.Wait(5000)) { continue }
        $count = $task.Result.Count
        if ($count -eq 0) { break }
        $text += [System.Text.Encoding]::UTF8.GetString($buffer, 0, $count)
        if ($task.Result.EndOfMessage) {
            $msg = $text | ConvertFrom-Json
            if ($msg.id -eq $wantId) { return $msg }
            $text = ''
        }
    }
    return $null
}

Send-Cdp 1 'Page.enable' @{}
[void](Read-Cdp 1)

Send-Cdp 2 'Page.captureScreenshot' @{ format = 'png'; captureBeyondViewport = $true }
$msg = Read-Cdp 2 40
$ws.Dispose()

if (-not $msg -or -not $msg.result.data) { throw 'engine returned no image' }
[System.IO.File]::WriteAllBytes($SavePath, [System.Convert]::FromBase64String($msg.result.data))
Write-Output ("saved: " + $SavePath + " (" + (Get-Item $SavePath).Length + " bytes)")
