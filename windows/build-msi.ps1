param(
    [string]$Gtk = "C:\gtk-build\gtk\x64\release",
    [string]$Version = "0.1.0"
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$env:PKG_CONFIG_PATH = "$Gtk\lib\pkgconfig"
$env:Path = "$Gtk\bin;$env:Path"
$env:LIB = "$Gtk\lib;$env:LIB"
$env:INCLUDE = "$Gtk\include;$env:INCLUDE"

cargo build --release --locked -p fluidityd -p fluidity-cli --manifest-path "$root\Cargo.toml"
if ($LASTEXITCODE) { exit $LASTEXITCODE }
cargo build --release --locked --manifest-path "$root\linux\Cargo.toml"
if ($LASTEXITCODE) { exit $LASTEXITCODE }

$out = "$root\dist\windows"
$stage = "$out\stage"
$gtkStage = "$stage\gtk"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
foreach ($dir in "bin", "lib", "share\glib-2.0\schemas", "share\icons\hicolor\scalable\apps", "share\icons\hicolor\symbolic\apps") {
    New-Item -ItemType Directory -Force "$gtkStage\$dir" | Out-Null
}

Copy-Item "$root\target\release\fluidityd.exe", "$root\target\release\fluidity.exe" $stage
Copy-Item "$root\linux\target\release\fluidity-gtk.exe" "$gtkStage\bin\Fluidity.exe"
Copy-Item "$Gtk\bin\*.dll" "$gtkStage\bin"
Get-ChildItem "$Gtk\bin" -Include "gdbus.exe", "gspawn-win64-helper*.exe" -Recurse | Copy-Item -Destination "$gtkStage\bin"
Copy-Item -Recurse "$Gtk\lib\gdk-pixbuf-2.0" "$gtkStage\lib"
Copy-Item -Recurse "$Gtk\share\icons\Adwaita" "$gtkStage\share\icons"
if (Test-Path "$Gtk\share\icons\hicolor\index.theme") { Copy-Item "$Gtk\share\icons\hicolor\index.theme" "$gtkStage\share\icons\hicolor" }
Copy-Item "$root\linux\data\icons\org.fluidityvpn.Fluidity.svg" "$gtkStage\share\icons\hicolor\scalable\apps"
Copy-Item "$root\linux\data\icons\org.fluidityvpn.Fluidity-symbolic.svg" "$gtkStage\share\icons\hicolor\symbolic\apps"
glib-compile-schemas --targetdir="$gtkStage\share\glib-2.0\schemas" "$Gtk\share\glib-2.0\schemas"
if ($LASTEXITCODE) { exit $LASTEXITCODE }

$text = (Get-Content -Raw "$root\LICENSE") -replace '\\', '\\' -replace '\{', '\{' -replace '\}', '\}' -replace "`r?`n", "\par`n"
Set-Content -Encoding Ascii "$stage\license.rtf" "{\rtf1\ansi\deff0{\fonttbl{\f0 Consolas;}}\f0\fs16 $text}"

wix build -arch x64 -ext WixToolset.UI.wixext -d "Version=$Version" -d "StageDir=$stage" `
    -o "$out\Fluidity-$Version-x64.msi" "$PSScriptRoot\Fluidity.wxs"
if ($LASTEXITCODE) { exit $LASTEXITCODE }
Write-Output "$out\Fluidity-$Version-x64.msi"
