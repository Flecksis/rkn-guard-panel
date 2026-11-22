$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
try {
    $release = Invoke-RestMethod -Uri 'https://api.github.com/repos/Flecksis/rkn-guard-panel/releases/latest' `
        -Headers @{ Accept = 'application/vnd.github+json' } -UserAgent 'rkn-guard-panel' -TimeoutSec 15
    $release | Select-Object tag_name, assets | ConvertTo-Json -Depth 6 -Compress
} catch {
    if ($_.Exception.Response -and [int]$_.Exception.Response.StatusCode -eq 404) {
        'null'
        exit 0
    }
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
