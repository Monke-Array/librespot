param(
    [Parameter(Mandatory = $true, ParameterSetName = "Inline")]
    [string] $Expression,

    [Parameter(Mandatory = $true, ParameterSetName = "File")]
    [string] $ExpressionPath,

    [int] $Port = 9222
)

$ErrorActionPreference = "Stop"

if ($PSCmdlet.ParameterSetName -eq "File") {
    $Expression = Get-Content -Raw -LiteralPath $ExpressionPath
}

$targets = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/json/list" -TimeoutSec 5
$target = $targets | Where-Object { $_.type -eq "page" } | Select-Object -First 1
if ($null -eq $target) {
    throw "No Spotify DevTools page target is available on port $Port"
}

$socket = [System.Net.WebSockets.ClientWebSocket]::new()
$cancellation = [Threading.CancellationToken]::None
$null = $socket.ConnectAsync([Uri] $target.webSocketDebuggerUrl, $cancellation).GetAwaiter().GetResult()

try {
    $request = @{
        id = 1
        method = "Runtime.evaluate"
        params = @{
            expression = $Expression
            awaitPromise = $true
            returnByValue = $true
        }
    } | ConvertTo-Json -Compress -Depth 20

    $requestBytes = [Text.Encoding]::UTF8.GetBytes($request)
    $requestSegment = [ArraySegment[byte]]::new($requestBytes)
    $null = $socket.SendAsync(
        $requestSegment,
        [System.Net.WebSockets.WebSocketMessageType]::Text,
        $true,
        $cancellation
    ).GetAwaiter().GetResult()

    $buffer = New-Object byte[] 65536
    $responseBytes = [Collections.Generic.List[byte]]::new()
    do {
        $segment = [ArraySegment[byte]]::new($buffer)
        $received = $socket.ReceiveAsync($segment, $cancellation).GetAwaiter().GetResult()
        if ($received.MessageType -eq [System.Net.WebSockets.WebSocketMessageType]::Close) {
            throw "Spotify DevTools closed the connection before replying"
        }
        for ($index = 0; $index -lt $received.Count; $index++) {
            $responseBytes.Add($buffer[$index])
        }
    } while (-not $received.EndOfMessage)

    $response = [Text.Encoding]::UTF8.GetString($responseBytes.ToArray()) | ConvertFrom-Json
    if ($null -ne $response.error) {
        throw ($response.error | ConvertTo-Json -Compress -Depth 20)
    }
    if ($null -ne $response.result.exceptionDetails) {
        throw ($response.result.exceptionDetails | ConvertTo-Json -Compress -Depth 20)
    }

    $response.result.result.value | ConvertTo-Json -Depth 100
}
finally {
    if ($socket.State -eq [System.Net.WebSockets.WebSocketState]::Open) {
        $null = $socket.CloseAsync(
            [System.Net.WebSockets.WebSocketCloseStatus]::NormalClosure,
            "done",
            $cancellation
        ).GetAwaiter().GetResult()
    }
    $socket.Dispose()
}
