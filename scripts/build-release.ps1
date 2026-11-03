# Сборка оптимизированной версии и упаковка в dist\Nebula (exe + WebView2Loader.dll).
# Запуск:  powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Set-Location $root

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "сборка не удалась" }

$dist = Join-Path $root "dist\Nebula"
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item "target\release\nebula-player.exe" (Join-Path $dist "Nebula.exe") -Force
# на GNU-тулчейне WebView2 подгружается через эту DLL — она должна лежать рядом с exe
Copy-Item "vendor\x64\WebView2Loader.dll" $dist -Force
Write-Output "Готово: $dist"
Get-ChildItem $dist | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } } | Format-Table -AutoSize
