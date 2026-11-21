# Compila el instalador firmado de WiZ Tray y el latest.json que lee el actualizador.
# Uso: .\scripts\release.ps1            -> deja todo en dist\
#      .\scripts\release.ps1 -Publish   -> además crea la release vX.Y.Z en GitHub con gh
param([switch]$Publish)

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$repo = 'dbqp01/wiz-tray'
$target = 'x86_64-pc-windows-gnu'

# La clave privada vive fuera del repo; sin ella el actualizador rechazaría la versión.
$key = Join-Path $env:USERPROFILE '.tauri\wiz-tray.key'
if (-not (Test-Path -LiteralPath $key)) { throw "Falta la clave de firma en $key. Créala con: cargo tauri signer generate -w `"$key`"" }
$env:TAURI_SIGNING_PRIVATE_KEY = $key
if (-not $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = '' }

$toolchain = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64'
$bin = Join-Path $toolchain 'bin'
$linker = Join-Path $bin 'x86_64-w64-mingw32-gcc.exe'
if (-not (Test-Path -LiteralPath $linker)) { throw 'No encuentro WinLibs GCC. Instala el paquete BrechtSanders.WinLibs.POSIX.UCRT.' }
$env:Path = "$bin;$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = $linker
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = '-C target-feature=+crt-static'

$version = (Get-Content -Raw (Join-Path $root 'src-tauri\tauri.conf.json') | ConvertFrom-Json).version
$tag = "v$version"

Push-Location (Join-Path $root 'src-tauri')
try {
    & cargo tauri build --target $target --bundles nsis
    if ($LASTEXITCODE -ne 0) { throw 'No se pudo compilar el instalador.' }
}
finally {
    Pop-Location
}

$nsis = Join-Path $root "src-tauri\target\$target\release\bundle\nsis"
$built = Get-ChildItem -LiteralPath $nsis -Filter "*_${version}_x64-setup.exe" | Select-Object -First 1
if (-not $built) { throw "No encuentro el instalador $version en $nsis." }

# Nombre sin espacios: GitHub cambia los espacios de los adjuntos por puntos y rompería la URL.
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Path $dist -Force | Out-Null
$setup = "WiZ-Tray_${version}_x64-setup.exe"
Copy-Item -LiteralPath $built.FullName -Destination (Join-Path $dist $setup) -Force
Copy-Item -LiteralPath "$($built.FullName).sig" -Destination (Join-Path $dist "$setup.sig") -Force

$notesFile = Join-Path $root "docs\releases\$tag.md"
$notes = if (Test-Path -LiteralPath $notesFile) { (Get-Content -Raw -LiteralPath $notesFile).Trim() } else { "WiZ Tray $version" }
$manifest = [ordered]@{
    version   = $version
    notes     = $notes
    pub_date  = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = [ordered]@{
        'windows-x86_64' = [ordered]@{
            signature = (Get-Content -Raw -LiteralPath (Join-Path $dist "$setup.sig")).Trim()
            url       = "https://github.com/$repo/releases/download/$tag/$setup"
        }
    }
}
# UTF-8 sin BOM: el actualizador no acepta el BOM que añade Set-Content en Windows PowerShell.
[IO.File]::WriteAllText((Join-Path $dist 'latest.json'), ($manifest | ConvertTo-Json -Depth 4), (New-Object Text.UTF8Encoding $false))

Write-Output "Instalador: $(Join-Path $dist $setup)"
Write-Output "Manifiesto: $(Join-Path $dist 'latest.json')"

if ($Publish) {
    & gh release create $tag --repo $repo --title "WiZ Tray $version" --notes $notes `
        (Join-Path $dist $setup) (Join-Path $dist "$setup.sig") (Join-Path $dist 'latest.json')
    if ($LASTEXITCODE -ne 0) { throw "No se pudo publicar $tag." }
    Write-Output "Publicada: https://github.com/$repo/releases/tag/$tag"
}
