$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
try {
    Invoke-WebRequest -UseBasicParsing -Uri $env:RKN_UPDATE_URL -OutFile $env:RKN_UPDATE_FILE -TimeoutSec 120
    if ((Get-Item -LiteralPath $env:RKN_UPDATE_FILE).Length -ne [long]$env:RKN_UPDATE_SIZE) {
        throw 'Размер скачанного EXE отличается от размера в релизе.'
    }
    if ((Get-FileHash -LiteralPath $env:RKN_UPDATE_FILE -Algorithm SHA256).Hash -ne $env:RKN_UPDATE_SHA256) {
        throw 'Контрольная сумма EXE не совпала.'
    }
    $stream = [IO.File]::OpenRead($env:RKN_UPDATE_FILE)
    try {
        if ($stream.ReadByte() -ne 77 -or $stream.ReadByte() -ne 90) {
            throw 'В релизе нет исполняемого файла Windows.'
        }
    } finally {
        $stream.Dispose()
    }
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
