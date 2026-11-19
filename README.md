# Formato Shorts 9:16

Aplicación de escritorio en Rust para convertir cualquier video a formato vertical **9:16**
(YouTube Shorts, Instagram Reels, TikTok), con subtítulos automáticos generados en tu PC.

## Funciones

- **Explorador de carpetas integrado**: abre una carpeta y ve sus videos con miniatura,
  duración, resolución y tamaño. Navega por subcarpetas, busca por nombre y agrega a la cola
  uno, varios o todos. Recuerda la última carpeta abierta.
- **Tres modos de ajuste**: fondo desenfocado, recortar (con posición izquierda/centro/derecha)
  o barras negras. Salida en 1080×1920 o 720×1280.
- **Recorte por tiempo**: inicio y fin por video (`90`, `1:30`, `1:02:03.5`), y botón
  «Otro clip» para sacar varios fragmentos del mismo video.
- **Subtítulos automáticos** con [whisper.cpp](https://github.com/ggerganov/whisper.cpp)
  (integrado en ffmpeg), sin internet ni costo: idioma, precisión (3 modelos), tamaño,
  posición, color y mayúsculas. Se quedan grabados en el video y opcionalmente en un `.srt`.
- **Modo «solo subtítulos»**: genera únicamente el `.srt` del video original.
- **Aceleración por GPU**: usa NVIDIA NVENC, Intel Quick Sync o AMD AMF si están disponibles;
  si la GPU falla, reintenta automáticamente con la CPU (x264).
- Cola de trabajo con barra de progreso, cancelar y arrastrar y soltar archivos o carpetas.

## Requisitos

- Windows 10/11.
- **ffmpeg** «full build» (incluye whisper, libass y codificadores por GPU) en el `PATH`:

  ```
  winget install Gyan.FFmpeg
  ```

- Los modelos de whisper se descargan desde la app la primera vez (148–574 MB) y se guardan
  en `%APPDATA%\FormatoShorts\models`.

## Compilar

```
cargo build --release
```

El ejecutable queda en `target\release\formato_shorts.exe`.

Pruebas:

```
cargo test
```
