$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$toolchain = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64'
$bin = Join-Path $toolchain 'bin'
$linker = Join-Path $bin 'x86_64-w64-mingw32-gcc.exe'
if (-not (Test-Path -LiteralPath $linker)) { throw 'No encuentro WinLibs GCC. Instala el paquete BrechtSanders.WinLibs.POSIX.UCRT.' }

$env:Path = "$bin;$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $linker
$env:RUSTFLAGS = '-C target-feature=+crt-static'
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
    & $cargo build --release --target $target
    if ($LASTEXITCODE -ne 0) { throw 'No se pudo compilar WiZ Tray.' }

    $exe = Join-Path $projectRoot "target\$target\release\wiz-tray.exe"
    $output = Join-Path $distDir 'WiZ Tray.exe'
    Copy-Item -LiteralPath $exe -Destination $output -Force
    $config = Join-Path $distDir 'wiz-tray.json'
    if (-not (Test-Path -LiteralPath $config)) {
        Copy-Item -LiteralPath (Join-Path $projectRoot 'wiz-tray.example.json') -Destination $config
    }
    Write-Output "Ejecutable listo: $output"
}
finally {
    Pop-Location
}
