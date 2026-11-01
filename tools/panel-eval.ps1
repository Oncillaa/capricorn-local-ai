param(
    [string]$Expression = '',
    [string]$FromFile = '',
    [int]$Port = 9222,
    [int]$TimeoutSec = 20
)

# Выполнить JS в открытой панели через протокол отладки WebView2.
# Панель — обычный WebView2, поэтому живёт она под тем же CDP, что и Chrome.

$ErrorActionPreference = 'Stop'
if ($FromFile) { $Expression = [System.IO.File]::ReadAllText($FromFile) }

$targets = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json" -TimeoutSec 8
$page = $targets | Where-Object { $_.type -eq 'page' } | Select-Object -First 1
if (-not $page) { throw "нет открытой страницы на порту $Port" }

$ws = New-Object System.Net.WebSockets.ClientWebSocket
$ct = [System.Threading.CancellationToken]::None
$ws.ConnectAsync([Uri]$page.webSocketDebuggerUrl, $ct).Wait(8000) | Out-Null

$payload = @{
    id     = 1
    method = 'Runtime.evaluate'
    params = @{
        expression    = $Expression
        returnByValue = $true
        awaitPromise  = $true
    }
} | ConvertTo-Json -Depth 6 -Compress

$bytes = [System.Text.Encoding]::UTF8.GetBytes($payload)
$segment = New-Object System.ArraySegment[byte] -ArgumentList @(,$bytes)
$ws.SendAsync($segment, [System.Net.WebSockets.WebSocketMessageType]::Text, $true, $ct).Wait(8000) | Out-Null

$buffer = New-Object byte[] 65536
$deadline = (Get-Date).AddSeconds($TimeoutSec)
$text = ''
while ((Get-Date) -lt $deadline) {
    $seg = New-Object System.ArraySegment[byte] -ArgumentList @(,$buffer)
    $task = $ws.ReceiveAsync($seg, $ct)
    if (-not $task.Wait(3000)) { continue }
    $count = $task.Result.Count
    if ($count -eq 0) { break }
    $text += [System.Text.Encoding]::UTF8.GetString($buffer, 0, $count)
    if ($task.Result.EndOfMessage) {
        $msg = $text | ConvertFrom-Json
        if ($msg.id -eq 1) { break }
        $text = ''
    }
}
$ws.Dispose()

$msg = $text | ConvertFrom-Json
if ($msg.result -and $msg.result.exceptionDetails) {
    Write-Output ("ОШИБКА JS: " + $msg.result.exceptionDetails.text)
    if ($msg.result.exceptionDetails.exception) {
        Write-Output $msg.result.exceptionDetails.exception.description
    }
    exit 1
}
if ($msg.result -and $msg.result.result) {
    $v = $msg.result.result.value
    if ($null -eq $v) { Write-Output 'null' } else { Write-Output $v }
} else {
    Write-Output $text
}
