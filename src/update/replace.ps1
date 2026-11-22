$ErrorActionPreference = 'Stop'
$target = $env:RKN_UPDATE_TARGET
$directory = $env:RKN_UPDATE_DIR
$incoming = Join-Path $directory 'new.exe'
$backup = Join-Path $directory 'old.exe'
$replaced = $false
$closed = $false

function Open-Panel {
    $start = [Diagnostics.ProcessStartInfo]::new($target)
    $start.WorkingDirectory = [IO.Path]::GetDirectoryName($target)
    $start.UseShellExecute = $true
    $process = [Diagnostics.Process]::Start($start)
    if (-not $process) { throw 'Не удалось открыть панель.' }
    $process.Dispose()
}

try {
    $old = Get-Process -Id ([int]$env:RKN_UPDATE_PID) -ErrorAction SilentlyContinue
    if ($old -and -not $old.WaitForExit(45000)) {
        throw 'Панель не завершилась. EXE остался прежним.'
    }
    $closed = $true
    if ((Get-FileHash -LiteralPath $incoming -Algorithm SHA256).Hash -ne $env:RKN_UPDATE_SHA256) {
        throw 'Контрольная сумма EXE изменилась перед установкой.'
    }
    for ($attempt = 0; ; $attempt++) {
        try {
            [IO.File]::Replace($incoming, $target, $backup)
            break
        } catch [IO.IOException] {
            if ($attempt -ge 10) { throw }
            Start-Sleep -Milliseconds 300
        }
    }
    $replaced = $true
    Open-Panel
} catch {
    $reason = $_.Exception.Message
    if ($replaced -and [IO.File]::Exists($backup)) {
        try { [IO.File]::Replace($backup, $target, [NullString]::Value) }
        catch { $reason += " Резервная копия: $backup" }
    }
    try { [IO.File]::WriteAllText("$target.update-error", $reason) } catch {}
    if ($closed) {
        try { Open-Panel } catch {}
    }
} finally {
    if ($replaced -and $reason -and [IO.File]::Exists($backup)) {
        # Если откат не удался, сохраняем старый EXE.
    } else {
        [IO.File]::Delete($backup)
    }
    [IO.File]::Delete($incoming)
    try { [IO.Directory]::Delete($directory) } catch {}
}
