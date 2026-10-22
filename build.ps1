$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$toolchain = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64'
$bin = Join-Path $toolchain 'bin'
$linker = Join-Path $bin 'x86_64-w64-mingw32-gcc.exe'
if (-not (Test-Path -LiteralPath $linker)) { throw 'No encuentro WinLibs GCC. Instala el paquete BrechtSanders.WinLibs.POSIX.UCRT.' }

$env:Path = "$bin;$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $linker
# Los flags van por target: con RUSTFLAGS global cargo los aplica tambien a los
# proc-macro del host y la compilacion de las pruebas falla.
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = '-C target-feature=+crt-static'
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
$target = 'x86_64-pc-windows-gnu'
$distDir = Join-Path $projectRoot 'dist'
New-Item -ItemType Directory -Path $distDir -Force | Out-Null

Push-Location $projectRoot
try {
    & $cargo fmt -- --check
    if ($LASTEXITCODE -ne 0) { throw 'cargo fmt encontró formato pendiente.' }
    & $cargo test --target $target
    if ($LASTEXITCODE -ne 0) { throw 'Fallaron las pruebas de WiZ Tray.' }

    Push-Location (Join-Path $projectRoot 'src-tauri')
    try {
        & $cargo fmt -- --check
        if ($LASTEXITCODE -ne 0) { throw 'cargo fmt encontró formato pendiente en la app.' }
        & $cargo test --target $target
        if ($LASTEXITCODE -ne 0) { throw 'Fallaron las pruebas de la interfaz.' }
        & $cargo build --release --target $target
        if ($LASTEXITCODE -ne 0) { throw 'No se pudo compilar WiZ Tray.' }

        $release = Join-Path $projectRoot "src-tauri\target\$target\release"
        Copy-Item -LiteralPath (Join-Path $release 'wiz-tray-app.exe') -Destination (Join-Path $distDir 'WiZ Tray.exe') -Force
        Copy-Item -LiteralPath (Join-Path $release 'WebView2Loader.dll') -Destination $distDir -Force
    }
    finally {
        Pop-Location
    }

    Write-Output "Ejecutable listo: $(Join-Path $distDir 'WiZ Tray.exe')"
}
finally {
    Pop-Location
}
