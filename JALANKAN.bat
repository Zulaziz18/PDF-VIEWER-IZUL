@echo off
title PDF Studio Izul v6
color 0B
echo ================================================
echo    PDF STUDIO IZUL v6 - Editor PDF Offline
echo ================================================
echo.

where python >nul 2>&1
if errorlevel 1 (
    echo Python tidak ditemukan di komputer ini.
    echo PDF Studio Izul butuh Python HANYA untuk menjalankan
    echo server lokal kecil ^(tidak ada koneksi internet yang dipakai^).
    echo.
    echo Silakan install dari https://www.python.org/downloads/
    echo Centang "Add Python to PATH" saat instalasi.
    pause
    exit /b
)

echo Menyalakan server lokal di http://localhost:8743 ...
echo ^(Server ini HANYA bisa diakses dari komputer ini sendiri^)
echo.
echo Jangan tutup jendela ini selama memakai PDF Studio Izul.
echo Tutup jendela ini untuk MEMATIKAN aplikasi.
echo.

start "" cmd /c "timeout /t 2 /nobreak >nul && start "" http://localhost:8743/index.html"
python serve.py
