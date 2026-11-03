# Однократная настройка сборки Nebula на тулчейне x86_64-pc-windows-gnu (без Visual Studio).
#
# Почему это нужно: современные crates `windows-*` линкуются через raw-dylib, и rustc
# вызывает dlltool. В rust-mingw есть только GNU dlltool без ассемблера, поэтому берём
# LLVM-версию из официального компонента rustup `llvm-tools` и линкуем через rust-lld
# (GNU ld не понимает LLVM import-библиотеки — exe падал бы при старте).
#
# Запуск:  powershell -ExecutionPolicy Bypass -File scripts\setup-gnu.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"

rustup component add llvm-tools
$sysroot = (rustc --print sysroot).Trim()
$bin = Join-Path $sysroot "lib\rustlib\x86_64-pc-windows-gnu\bin"
if (-not (Test-Path (Join-Path $bin "llvm-ar.exe"))) { throw "llvm-ar.exe не найден в $bin" }

$tools = Join-Path $root ".tools"
New-Item -ItemType Directory -Force $tools | Out-Null
# llvm-ar, запущенный под именем dlltool, работает в режиме llvm-dlltool
Copy-Item (Join-Path $bin "llvm-ar.exe") (Join-Path $tools "dlltool.exe") -Force
foreach ($dll in "libgcc_s_seh-1.dll", "libwinpthread-1.dll") {
  $src = Join-Path $bin $dll
  if (Test-Path $src) { Copy-Item $src $tools -Force }
}

$lld = (Join-Path $bin "rust-lld.exe").Replace('\', '/')
$dlltool = (Join-Path $tools "dlltool.exe").Replace('\', '/')
$config = @"
# Сгенерировано scripts/setup-gnu.ps1 для тулчейна x86_64-pc-windows-gnu (без Visual Studio).
# Современные crates `windows-*` линкуются через raw-dylib, и rustc нужен dlltool.
# Используем LLVM-версию из компонента rustup `llvm-tools` и линковщик rust-lld:
# GNU ld из rust-mingw не умеет работать с LLVM import-библиотеками.
[target.x86_64-pc-windows-gnu]
linker = "$lld"
rustflags = [
  "-C", "linker-flavor=ld.lld",
  "-C", "dlltool=$dlltool",
]
"@
New-Item -ItemType Directory -Force (Join-Path $root ".cargo") | Out-Null
[IO.File]::WriteAllText((Join-Path $root ".cargo\config.toml"), $config, (New-Object Text.UTF8Encoding($false)))
Write-Output "Готово: .cargo\config.toml и .tools\dlltool.exe настроены."
