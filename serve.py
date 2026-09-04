"""
Server lokal kecil untuk PDF Studio Izul v6.
Menentukan Content-Type secara eksplisit (tidak bergantung pada
database mimetypes bawaan tiap versi Python) supaya file .mjs
(ES Module untuk pdf.js) selalu dikenali benar oleh browser.
"""
import http.server
import socketserver
import os

PORT = 8743

EXPLICIT_TYPES = {
    ".html": "text/html; charset=utf-8",
    ".js":   "text/javascript; charset=utf-8",
    ".mjs":  "text/javascript; charset=utf-8",
    ".css":  "text/css; charset=utf-8",
    ".pdf":  "application/pdf",
    ".json": "application/json",
    ".png":  "image/png",
    ".jpg":  "image/jpeg",
    ".jpeg": "image/jpeg",
    ".map":  "application/json",
    ".wasm": "application/wasm",
}


class Handler(http.server.SimpleHTTPRequestHandler):
    def guess_type(self, path):
        ext = os.path.splitext(path)[1].lower()
        if ext in EXPLICIT_TYPES:
            return EXPLICIT_TYPES[ext]
        # file chunk model AI (nama = hash, tanpa ekstensi) -> data biner umum
        base = os.path.basename(path)
        if "/vendor/bgremoval/" in path.replace("\\", "/") and "." not in base:
            return "application/octet-stream"
        return super().guess_type(path)

    def end_headers(self):
        # Matikan cache browser sepenuhnya. Tanpa ini, Chrome/Edge kerap
        # menyimpan versi lama app.js/core.js/annots.js/style.css dan
        # terus memakainya walau file di disk sudah ditimpa dengan yang
        # baru - user seolah "revisinya tidak masuk" padahal filenya benar.
        self.send_header("Cache-Control", "no-store, no-cache, must-revalidate, max-age=0")
        self.send_header("Pragma", "no-cache")
        self.send_header("Expires", "0")
        super().end_headers()

    def log_message(self, fmt, *args):
        pass  # jangan spam terminal dengan log tiap request


if __name__ == "__main__":
    os.chdir(os.path.dirname(os.path.abspath(__file__)))
    with socketserver.TCPServer(("127.0.0.1", PORT), Handler) as httpd:
        print(f"PDF Studio Izul v6 berjalan di http://localhost:{PORT}")
        print("Tutup jendela ini untuk mematikan aplikasi.")
        httpd.serve_forever()
