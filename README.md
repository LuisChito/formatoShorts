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
- **Subtítulos automáticos estilo Shorts** con [whisper.cpp](https://github.com/ggerganov/whisper.cpp)
  (integrado en ffmpeg), sin internet ni costo:
  - tiempos **por palabra** sincronizados con la voz (se quitan los silencios antes de transcribir
    y cada palabra vuelve a su tiempo real),
  - estilos: resaltar la palabra que se dice (karaoke), palabra por palabra o frase simple,
  - animaciones: pop, deslizar, fundido,
  - fuentes incluidas: Luckiest Guy, Poppins Black y Anton (licencias en `assets/fonts`),
  - color de resaltado, tamaño, posición, mayúsculas y vista previa en vivo.
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

Modo línea de comandos (útil para automatizar):

```
FormatoShorts.exe --cli video.mp4 --clip 0:15-0:45 --clip 1:30-2:10 --clip 5:00-5:40
```

- `--clip INICIO-FIN` se puede repetir para sacar varios clips del mismo video. Se guardan en
  `<carpeta del video>\<nombre>_clips\` como `<nombre>_clip01_9x16.mp4`, `_clip02`… (con su `.srt`).
- `--salida CARPETA` cambia la carpeta de destino.
- `--inicio T --fin T` para un solo clip; `--sin-subs`, `--solo-srt`, `--cpu`.
- Imprime `OK <ruta>` o `ERROR ...` por clip; el código de salida es 0 si todos salieron bien.

Pruebas:

```
cargo test
```
