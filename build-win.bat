@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
if errorlevel 1 exit /b 1
set PATH=C:\Users\loicd\.cargo\bin;%PATH%
set LIBMPV_MPV_SOURCE=C:\libmpv
set LIBMPV2_MPV_SOURCE=C:\libmpv
set MPV_SOURCE=C:\libmpv
set RUSTFLAGS=-L C:\libmpv
set LIB=C:\libmpv;%LIB%
set INCLUDE=C:\libmpv\include;%INCLUDE%
cd /d D:\KoalaTV\desktop
cargo build --release --target x86_64-pc-windows-msvc %*
