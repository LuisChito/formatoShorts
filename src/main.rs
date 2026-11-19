// Formato Shorts: convierte cualquier video a formato vertical 9:16 usando ffmpeg,
// con explorador de carpetas, recorte por tiempo y subtítulos automáticos
// (whisper.cpp integrado en ffmpeg). También puede generar solo el .srt.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod subtitulos;

use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use subtitulos as st;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const EXTENSIONES: &[&str] = &[
    "mp4", "mov", "mkv", "avi", "webm", "flv", "wmv", "m4v", "mpg", "mpeg", "3gp", "ts",
];

// ───────────────────────────── Opciones ─────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum QueGenerar {
    Video,
    SoloSubtitulos,
}

#[derive(Clone, Copy, PartialEq)]
enum Modo {
    Recortar,
    FondoDesenfocado,
    BarrasNegras,
}

impl Modo {
    fn nombre(self) -> &'static str {
        match self {
            Modo::Recortar => "Recortar (llenar pantalla)",
            Modo::FondoDesenfocado => "Fondo desenfocado",
            Modo::BarrasNegras => "Barras negras",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Resolucion {
    R1080,
    R720,
}

impl Resolucion {
    fn dims(self) -> (u32, u32) {
        match self {
            Resolucion::R1080 => (1080, 1920),
            Resolucion::R720 => (720, 1280),
        }
    }
}

/// Codificadores por GPU que funcionan en este equipo (se detecta una sola vez).
static GPUS: std::sync::OnceLock<Vec<Codificador>> = std::sync::OnceLock::new();

fn gpus_disponibles() -> &'static [Codificador] {
    GPUS.get_or_init(|| {
        Codificador::GPUS
            .into_iter()
            .filter(|c| c.funciona())
            .collect()
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Aceleracion {
    Automatica, // GPU si hay; si falla, CPU
    SoloCpu,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Codificador {
    Nvenc,
    Qsv,
    Amf,
    Cpu,
}

impl Codificador {
    const GPUS: [Codificador; 3] = [Codificador::Nvenc, Codificador::Qsv, Codificador::Amf];

    fn nombre(self) -> &'static str {
        match self {
            Codificador::Nvenc => "GPU NVIDIA (NVENC)",
            Codificador::Qsv => "GPU Intel (Quick Sync)",
            Codificador::Amf => "GPU AMD (AMF)",
            Codificador::Cpu => "CPU (x264)",
        }
    }

    /// Decodificación por hardware que acompaña al codificador (antes de -i).
    fn hwaccel(self) -> Option<&'static str> {
        match self {
            // Con QSV/AMF la decodificación por hardware choca con los filtros de CPU,
            // así que solo se acelera la codificación.
            Codificador::Nvenc => Some("cuda"),
            Codificador::Qsv | Codificador::Amf | Codificador::Cpu => None,
        }
    }

    /// Argumentos de codificación; `crf` es la calidad elegida (16–30).
    fn args(self, crf: u32) -> Vec<String> {
        let v: Vec<String> = match self {
            Codificador::Nvenc => vec![
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p5",
                "-tune",
                "hq",
                "-rc",
                "vbr",
                "-cq",
                &(crf + 2).to_string(),
                "-b:v",
                "0",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            Codificador::Qsv => vec![
                "-c:v",
                "h264_qsv",
                "-preset",
                "medium",
                "-global_quality",
                &(crf + 2).to_string(),
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            Codificador::Amf => vec![
                "-c:v",
                "h264_amf",
                "-quality",
                "quality",
                "-rc",
                "cqp",
                "-qp_i",
                &crf.to_string(),
                "-qp_p",
                &(crf + 2).to_string(),
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            Codificador::Cpu => vec![
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-crf",
                &crf.to_string(),
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        };
        v
    }

    /// Prueba real: codifica medio segundo de video en negro.
    fn funciona(self) -> bool {
        let mut cmd = comando("ffmpeg");
        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=black:s=1080x1920:d=0.5",
        ]);
        cmd.args(self.args(20))
            .args(["-pix_fmt", "yuv420p", "-f", "null", "-"]);
        cmd.output().map(|o| o.status.success()).unwrap_or(false)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Modelo {
    Base,
    Small,
    Turbo,
}

impl Modelo {
    fn archivo(self) -> &'static str {
        match self {
            Modelo::Base => "ggml-base.bin",
            Modelo::Small => "ggml-small.bin",
            Modelo::Turbo => "ggml-large-v3-turbo-q5_0.bin",
        }
    }
    fn nombre(self) -> &'static str {
        match self {
            Modelo::Base => "Básico (148 MB, rápido)",
            Modelo::Small => "Bueno (488 MB)",
            Modelo::Turbo => "Excelente (574 MB, más lento)",
        }
    }
    fn bytes_aprox(self) -> u64 {
        match self {
            Modelo::Base => 147_951_465,
            Modelo::Small => 487_601_967,
            Modelo::Turbo => 574_041_195,
        }
    }
    fn url(self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.archivo()
        )
    }
    fn ruta(self) -> PathBuf {
        carpeta_app().join("models").join(self.archivo())
    }
    fn instalado(self) -> bool {
        self.ruta().is_file()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Idioma {
    Auto,
    Es,
    En,
    Pt,
}

impl Idioma {
    fn codigo(self) -> &'static str {
        match self {
            Idioma::Auto => "auto",
            Idioma::Es => "es",
            Idioma::En => "en",
            Idioma::Pt => "pt",
        }
    }
    fn nombre(self) -> &'static str {
        match self {
            Idioma::Auto => "Detectar automáticamente",
            Idioma::Es => "Español",
            Idioma::En => "Inglés",
            Idioma::Pt => "Portugués",
        }
    }
}

#[derive(Clone)]
struct OpcionesSubs {
    activar: bool,
    modelo: Modelo,
    idioma: Idioma,
    aspecto: st::Opciones,
    guardar_srt: bool,
}

#[derive(Clone)]
struct Config {
    que: QueGenerar,
    modo: Modo,
    resolucion: Resolucion,
    posicion_x: f32, // 0.0 = izquierda, 0.5 = centro, 1.0 = derecha (solo Recortar)
    calidad_crf: u32,
    aceleracion: Aceleracion,
    carpeta_salida: Option<PathBuf>,
    subs: OpcionesSubs,
}

impl Config {
    fn necesita_whisper(&self) -> bool {
        self.que == QueGenerar::SoloSubtitulos || self.subs.activar
    }
}

// ───────────────────────────── Estado ─────────────────────────────

#[derive(Clone, PartialEq)]
enum Estado {
    Pendiente,
    Procesando(&'static str, f32),
    Listo(PathBuf, String),
    Error(String),
    Cancelado,
}

#[derive(Clone)]
struct Trabajo {
    entrada: PathBuf,
    estado: Estado,
    duracion: Option<f64>,
    inicio: String,
    fin: String,
    /// Nombre del archivo de salida (sin extensión); si falta, se usa el del video.
    nombre: Option<String>,
}

impl Trabajo {
    fn nuevo(entrada: PathBuf) -> Self {
        let duracion = probar_video(&entrada).0;
        Self::con_duracion(entrada, duracion)
    }

    fn con_duracion(entrada: PathBuf, duracion: Option<f64>) -> Self {
        Self {
            entrada,
            estado: Estado::Pendiente,
            duracion,
            inicio: String::new(),
            fin: String::new(),
            nombre: None,
        }
    }

    /// Devuelve (inicio, duración del clip) en segundos, o un mensaje de error.
    fn rango(&self) -> Result<(Option<f64>, Option<f64>), String> {
        let ini = parsear_tiempo(&self.inicio).map_err(|_| "Inicio no válido".to_string())?;
        let fin = parsear_tiempo(&self.fin).map_err(|_| "Fin no válido".to_string())?;
        if let Some(d) = self.duracion {
            if ini.is_some_and(|i| i >= d) {
                return Err(format!(
                    "El inicio supera la duración ({})",
                    formatear_tiempo(d)
                ));
            }
            if fin.is_some_and(|f| f > d + 0.05) {
                return Err(format!(
                    "El fin supera la duración ({})",
                    formatear_tiempo(d)
                ));
            }
        }
        let largo = match (ini, fin) {
            (Some(i), Some(f)) if f <= i => {
                return Err("El fin debe ser mayor que el inicio".into());
            }
            (i, Some(f)) => Some(f - i.unwrap_or(0.0)),
            _ => None,
        };
        Ok((ini.filter(|i| *i > 0.0), largo))
    }
}

#[derive(Default)]
struct Compartido {
    trabajos: Vec<Trabajo>,
    en_curso: bool,
    cancelar: bool,
    hijo: Option<Child>,
}

#[derive(Clone, PartialEq)]
enum Descarga {
    Nada,
    Descargando(Modelo, f32),
    Error(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Pestana {
    Carpeta,
    Cola,
}

/// Un video encontrado en la carpeta que se está explorando.
struct ArchivoVideo {
    ruta: PathBuf,
    nombre: String,
    bytes: u64,
}

/// Datos que se calculan en segundo plano para cada video de la carpeta.
#[derive(Default)]
struct InfoVideo {
    duracion: Option<f64>,
    dims: Option<(u32, u32)>,
    imagen: Option<egui::ColorImage>,
}

struct App {
    config: Config,
    compartido: Arc<Mutex<Compartido>>,
    descarga: Arc<Mutex<Descarga>>,
    ffmpeg_ok: bool,
    whisper_ok: bool,
    // Explorador de carpetas
    pestana: Pestana,
    carpeta: Option<PathBuf>,
    subcarpetas: Vec<PathBuf>,
    videos: Vec<ArchivoVideo>,
    busqueda: String,
    seleccion: HashSet<PathBuf>,
    info: Arc<Mutex<HashMap<PathBuf, InfoVideo>>>,
    texturas: HashMap<PathBuf, egui::TextureHandle>,
    generacion: Arc<AtomicU64>,
    aviso: Option<String>,
    // Vista previa del estilo de subtítulos
    previa_imagen: Arc<Mutex<Option<egui::ColorImage>>>,
    previa_textura: Option<egui::TextureHandle>,
    previa_clave: String,
    previa_ocupada: Arc<AtomicBool>,
}

// ───────────────────────────── Utilidades ─────────────────────────────

/// Acepta "", "90", "90.5", "1:30", "01:02:03.25".
fn parsear_tiempo(texto: &str) -> Result<Option<f64>, ()> {
    let t = texto.trim().replace(',', ".");
    if t.is_empty() {
        return Ok(None);
    }
    let partes: Vec<&str> = t.split(':').collect();
    if partes.len() > 3 {
        return Err(());
    }
    let mut total = 0.0;
    for (i, p) in partes.iter().enumerate() {
        let v: f64 = p.parse().map_err(|_| ())?;
        if !v.is_finite() || v < 0.0 || (i > 0 && v >= 60.0) {
            return Err(());
        }
        total = total * 60.0 + v;
    }
    Ok(Some(total))
}

fn formatear_tiempo(seg: f64) -> String {
    let s = seg.max(0.0);
    let h = (s / 3600.0) as u64;
    let m = ((s % 3600.0) / 60.0) as u64;
    let r = s % 60.0;
    if h > 0 {
        format!("{h}:{m:02}:{r:04.1}")
    } else {
        format!("{m}:{r:04.1}")
    }
}

fn formatear_bytes(b: u64) -> String {
    let mb = b as f64 / 1_048_576.0;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{mb:.0} MB")
    }
}

/// Carpeta de datos de la app: %APPDATA%\FormatoShorts
fn carpeta_app() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("FormatoShorts")
}

fn archivo_ultima_carpeta() -> PathBuf {
    carpeta_app().join("ultima_carpeta.txt")
}

/// Crea un Command sin abrir ventana de consola en Windows.
fn comando(programa: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(programa);
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

fn es_video(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONES.contains(&e.to_ascii_lowercase().as_str()))
}

/// Duración (s) y resolución del primer stream de video.
fn probar_video(p: &Path) -> (Option<f64>, Option<(u32, u32)>) {
    let Ok(out) = comando("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args([
            "-show_entries",
            "stream=width,height:format=duration",
            "-of",
            "default=nw=1",
        ])
        .arg(p)
        .output()
    else {
        return (None, None);
    };
    let texto = String::from_utf8_lossy(&out.stdout);
    let (mut dur, mut w, mut h) = (None, None, None);
    for l in texto.lines() {
        if let Some(v) = l.strip_prefix("duration=") {
            dur = v.trim().parse().ok();
        } else if let Some(v) = l.strip_prefix("width=") {
            w = v.trim().parse().ok();
        } else if let Some(v) = l.strip_prefix("height=") {
            h = v.trim().parse().ok();
        }
    }
    (dur, w.zip(h))
}

/// Genera (o reutiliza de la caché) una miniatura JPG y la carga como imagen.
fn miniatura(p: &Path, duracion: Option<f64>) -> Option<egui::ColorImage> {
    let meta = std::fs::metadata(p).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    p.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    if let Ok(m) = meta.modified() {
        m.hash(&mut hasher);
    }
    let dir = carpeta_app().join("thumbs");
    let _ = std::fs::create_dir_all(&dir);
    let jpg = dir.join(format!("{:016x}.jpg", hasher.finish()));

    if !jpg.is_file() {
        let t = duracion.map(|d| (d * 0.1).min(5.0)).unwrap_or(0.0);
        let _ = comando("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{t:.2}"), "-i"])
            .arg(p)
            .args(["-frames:v", "1", "-vf"])
            .arg("scale=320:180:force_original_aspect_ratio=decrease,pad=320:180:(ow-iw)/2:(oh-ih)/2:black")
            .args(["-q:v", "4"])
            .arg(&jpg)
            .output();
    }
    let img = image::open(&jpg).ok()?.to_rgba8();
    let tam = [img.width() as usize, img.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(tam, img.as_raw()))
}

fn abrir_con_sistema(p: &Path) {
    let _ = Command::new("explorer").arg(p).spawn();
}

fn mostrar_en_explorador(p: &Path) {
    let _ = Command::new("explorer")
        .arg(format!("/select,{}", p.display()))
        .spawn();
}

fn construir_filtro(config: &Config, ass_relativo: Option<&str>) -> String {
    let (w, h) = config.resolucion.dims();
    let base = match config.modo {
        Modo::Recortar => format!(
            "[0:v]format=yuv420p,scale={w}:{h}:force_original_aspect_ratio=increase,\
             crop={w}:{h}:(iw-ow)*{px:.3}:(ih-oh)/2,setsar=1",
            px = config.posicion_x
        ),
        Modo::BarrasNegras => format!(
            "[0:v]format=yuv420p,scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:black,setsar=1"
        ),
        Modo::FondoDesenfocado => format!(
            "[0:v]format=yuv420p,split=2[bg][fg];\
             [bg]scale={wb}:{hb}:force_original_aspect_ratio=increase,crop={wb}:{hb},boxblur=8:3,scale={w}:{h}[bg2];\
             [fg]scale={w}:{h}:force_original_aspect_ratio=decrease[fg2];\
             [bg2][fg2]overlay=(W-w)/2:(H-h)/2,setsar=1",
            wb = w / 4,
            hb = h / 4
        ),
    };
    let subs = match ass_relativo {
        Some(ass) => format!(",ass=filename={ass}:fontsdir=fonts"),
        None => String::new(),
    };
    format!("{base}{subs},format=yuv420p[v]")
}

/// Ruta de salida única: <carpeta>/<nombre><sufijo>.<ext>
fn ruta_salida(trabajo: &Trabajo, config: &Config, sufijo: &str, ext: &str) -> PathBuf {
    let entrada = &trabajo.entrada;
    let carpeta = config
        .carpeta_salida
        .clone()
        .unwrap_or_else(|| entrada.parent().map(Path::to_path_buf).unwrap_or_default());
    let base = trabajo.nombre.clone().unwrap_or_else(|| {
        entrada
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "video".into())
    });
    let mut ruta = carpeta.join(format!("{base}{sufijo}.{ext}"));
    let mut n = 2;
    while ruta.exists() {
        ruta = carpeta.join(format!("{base}{sufijo}_{n}.{ext}"));
        n += 1;
    }
    ruta
}

// ───────────────────────────── Proceso ─────────────────────────────

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let ffmpeg_ok = comando("ffmpeg").arg("-version").output().is_ok()
            && comando("ffprobe").arg("-version").output().is_ok();
        let whisper_ok = ffmpeg_ok
            && comando("ffmpeg")
                .args(["-hide_banner", "-filters"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains("whisper"))
                .unwrap_or(false);
        {
            let ctx = ctx.clone();
            thread::spawn(move || {
                gpus_disponibles();
                ctx.request_repaint();
            });
        }
        // Elegir por defecto el mejor modelo ya descargado
        let modelo = mejor_modelo();
        let mut app = Self {
            config: config_por_defecto(modelo),
            compartido: Arc::new(Mutex::new(Compartido::default())),
            descarga: Arc::new(Mutex::new(Descarga::Nada)),
            ffmpeg_ok,
            whisper_ok,
            pestana: Pestana::Carpeta,
            carpeta: None,
            subcarpetas: Vec::new(),
            videos: Vec::new(),
            busqueda: String::new(),
            seleccion: HashSet::new(),
            info: Arc::new(Mutex::new(HashMap::new())),
            texturas: HashMap::new(),
            generacion: Arc::new(AtomicU64::new(0)),
            aviso: None,
            previa_imagen: Arc::new(Mutex::new(None)),
            previa_textura: None,
            previa_clave: String::new(),
            previa_ocupada: Arc::new(AtomicBool::new(false)),
        };
        if let Ok(t) = std::fs::read_to_string(archivo_ultima_carpeta()) {
            let p = PathBuf::from(t.trim());
            if p.is_dir() {
                app.abrir_carpeta(p, ctx);
            }
        }
        app
    }

    fn abrir_carpeta(&mut self, dir: PathBuf, ctx: &egui::Context) {
        let mut subcarpetas = Vec::new();
        let mut videos = Vec::new();
        match std::fs::read_dir(&dir) {
            Ok(entradas) => {
                for e in entradas.flatten() {
                    let ruta = e.path();
                    let Ok(meta) = e.metadata() else { continue };
                    if meta.is_dir() {
                        subcarpetas.push(ruta);
                    } else if es_video(&ruta) {
                        let nombre = e.file_name().to_string_lossy().into_owned();
                        videos.push(ArchivoVideo {
                            ruta,
                            nombre,
                            bytes: meta.len(),
                        });
                    }
                }
            }
            Err(e) => {
                self.aviso = Some(format!("No se pudo abrir la carpeta: {e}"));
                return;
            }
        }
        subcarpetas.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
        videos.sort_by_key(|v| v.nombre.to_lowercase());

        let _ = std::fs::create_dir_all(carpeta_app());
        let _ = std::fs::write(archivo_ultima_carpeta(), dir.to_string_lossy().as_bytes());

        // Reiniciar caché y lanzar el cálculo de miniaturas en segundo plano
        let mi_gen = self.generacion.fetch_add(1, Ordering::SeqCst) + 1;
        self.info.lock().unwrap().clear();
        self.texturas.clear();
        self.seleccion.clear();
        self.busqueda.clear();
        self.aviso = None;
        let cola = Arc::new(Mutex::new(
            videos
                .iter()
                .rev()
                .map(|v| v.ruta.clone())
                .collect::<Vec<_>>(),
        ));
        for _ in 0..3 {
            let (cola, info, generacion, ctx) = (
                Arc::clone(&cola),
                Arc::clone(&self.info),
                Arc::clone(&self.generacion),
                ctx.clone(),
            );
            thread::spawn(move || {
                loop {
                    if generacion.load(Ordering::SeqCst) != mi_gen {
                        break;
                    }
                    let Some(p) = cola.lock().unwrap().pop() else {
                        break;
                    };
                    let (duracion, dims) = probar_video(&p);
                    let imagen = miniatura(&p, duracion);
                    if generacion.load(Ordering::SeqCst) != mi_gen {
                        break;
                    }
                    info.lock().unwrap().insert(
                        p,
                        InfoVideo {
                            duracion,
                            dims,
                            imagen,
                        },
                    );
                    ctx.request_repaint();
                }
            });
        }

        self.carpeta = Some(dir);
        self.subcarpetas = subcarpetas;
        self.videos = videos;
    }

    fn en_cola(&self) -> HashSet<PathBuf> {
        self.compartido
            .lock()
            .unwrap()
            .trabajos
            .iter()
            .map(|t| t.entrada.clone())
            .collect()
    }

    fn agregar_a_cola(&mut self, rutas: Vec<PathBuf>) {
        let ya = self.en_cola();
        let info = self.info.lock().unwrap();
        let nuevos: Vec<Trabajo> = rutas
            .into_iter()
            .filter(|p| !ya.contains(p))
            .map(|p| match info.get(&p) {
                Some(i) => Trabajo::con_duracion(p, i.duracion),
                None => Trabajo::nuevo(p),
            })
            .collect();
        drop(info);
        let n = nuevos.len();
        self.compartido.lock().unwrap().trabajos.extend(nuevos);
        if n > 0 {
            self.aviso = Some(format!(
                "✔ {n} video{} agregado{} a la cola",
                if n == 1 { "" } else { "s" },
                if n == 1 { "" } else { "s" }
            ));
        }
    }

    fn iniciar(&self, ctx: egui::Context) {
        let compartido = Arc::clone(&self.compartido);
        let config = self.config.clone();
        {
            let mut c = compartido.lock().unwrap();
            if c.en_curso {
                return;
            }
            c.en_curso = true;
            c.cancelar = false;
            for t in c.trabajos.iter_mut() {
                if !matches!(t.estado, Estado::Listo(..)) {
                    t.estado = Estado::Pendiente;
                }
            }
        }
        thread::spawn(move || procesar_cola(compartido, config, ctx));
    }

    fn descargar_modelo(&self, modelo: Modelo, ctx: egui::Context) {
        let estado = Arc::clone(&self.descarga);
        *estado.lock().unwrap() = Descarga::Descargando(modelo, 0.0);
        thread::spawn(move || {
            let destino = modelo.ruta();
            let parcial = destino.with_extension("part");
            let _ = std::fs::create_dir_all(destino.parent().unwrap());
            let hijo = comando("curl.exe")
                .args(["-L", "-s", "-f", "-o"])
                .arg(&parcial)
                .arg(modelo.url())
                .spawn();
            let mut hijo = match hijo {
                Ok(h) => h,
                Err(e) => {
                    *estado.lock().unwrap() =
                        Descarga::Error(format!("No se pudo ejecutar curl: {e}"));
                    ctx.request_repaint();
                    return;
                }
            };
            let resultado = loop {
                match hijo.try_wait() {
                    Ok(Some(s)) => break s.success(),
                    Ok(None) => {
                        let actual = std::fs::metadata(&parcial).map(|m| m.len()).unwrap_or(0);
                        let p = (actual as f32 / modelo.bytes_aprox() as f32).min(0.99);
                        *estado.lock().unwrap() = Descarga::Descargando(modelo, p);
                        ctx.request_repaint();
                        thread::sleep(Duration::from_millis(300));
                    }
                    Err(_) => break false,
                }
            };
            let ok = resultado && std::fs::rename(&parcial, &destino).is_ok();
            *estado.lock().unwrap() = if ok {
                Descarga::Nada
            } else {
                let _ = std::fs::remove_file(&parcial);
                Descarga::Error("La descarga falló. Revisa tu conexión a internet.".into())
            };
            ctx.request_repaint();
        });
    }
}

fn procesar_cola(compartido: Arc<Mutex<Compartido>>, config: Config, ctx: egui::Context) {
    loop {
        let siguiente = {
            let c = compartido.lock().unwrap();
            if c.cancelar {
                None
            } else {
                c.trabajos
                    .iter()
                    .position(|t| t.estado == Estado::Pendiente)
                    .map(|i| (i, c.trabajos[i].clone()))
            }
        };
        let Some((idx, trabajo)) = siguiente else {
            break;
        };

        let resultado = convertir(&compartido, idx, &trabajo, &config, &ctx);
        {
            let mut c = compartido.lock().unwrap();
            let cancelado = c.cancelar;
            if let Some(t) = c.trabajos.get_mut(idx) {
                t.estado = if cancelado {
                    Estado::Cancelado
                } else {
                    match resultado {
                        Ok((p, nota)) => Estado::Listo(p, nota),
                        Err(e) => Estado::Error(e),
                    }
                };
            }
        }
        ctx.request_repaint();
    }
    let mut c = compartido.lock().unwrap();
    c.en_curso = false;
    c.hijo = None;
    ctx.request_repaint();
}

/// Ejecuta ffmpeg mostrando el progreso de la etapa indicada.
fn ejecutar_ffmpeg(
    compartido: &Arc<Mutex<Compartido>>,
    idx: usize,
    mut cmd: Command,
    duracion: f64,
    etapa: &'static str,
    ctx: &egui::Context,
) -> Result<(), String> {
    if let Some(t) = compartido.lock().unwrap().trabajos.get_mut(idx) {
        t.estado = Estado::Procesando(etapa, 0.0);
    }
    ctx.request_repaint();

    let mut hijo = cmd
        .args(["-progress", "pipe:1", "-nostats", "-loglevel", "error"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("No se pudo ejecutar ffmpeg: {e}"))?;

    let stdout = hijo.stdout.take().unwrap();
    let stderr = hijo.stderr.take().unwrap();
    {
        let mut c = compartido.lock().unwrap();
        if c.cancelar {
            let _ = hijo.kill();
        }
        c.hijo = Some(hijo);
    }

    let lector_err = thread::spawn(move || {
        let mut s = String::new();
        for linea in BufReader::new(stderr).lines().map_while(Result::ok) {
            s.push_str(&linea);
            s.push('\n');
        }
        s
    });

    for linea in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(v) = linea.strip_prefix("out_time_us=") {
            if let (Ok(us), true) = (v.trim().parse::<f64>(), duracion > 0.0) {
                let p = ((us / 1_000_000.0) / duracion).clamp(0.0, 1.0) as f32;
                if let Some(t) = compartido.lock().unwrap().trabajos.get_mut(idx) {
                    t.estado = Estado::Procesando(etapa, p);
                }
                ctx.request_repaint();
            }
        }
    }

    let hijo = compartido.lock().unwrap().hijo.take();
    let estado = match hijo {
        Some(mut h) => h.wait().map_err(|e| e.to_string())?,
        None => return Err("Proceso perdido".into()),
    };
    let errores = lector_err.join().unwrap_or_default();
    if estado.success() {
        Ok(())
    } else {
        Err(errores.lines().last().unwrap_or("ffmpeg falló").to_string())
    }
}

/// Comando ffmpeg con el recorte por tiempo ya aplicado a la entrada.
fn ffmpeg_con_rango(
    entrada: &Path,
    inicio: Option<f64>,
    largo: Option<f64>,
    hwaccel: Option<&str>,
) -> Command {
    let mut cmd = comando("ffmpeg");
    cmd.args(["-hide_banner", "-y"]);
    if let Some(h) = hwaccel {
        cmd.args(["-hwaccel", h]);
    }
    if let Some(i) = inicio {
        cmd.args(["-ss", &format!("{i:.3}")]);
    }
    // -t como opción de ENTRADA: así los filtros (whisper) reciben el fin del audio
    // y procesan el último bloque; como opción de salida el .srt quedaría incompleto.
    if let Some(l) = largo {
        cmd.args(["-t", &format!("{l:.3}")]);
    }
    cmd.arg("-i").arg(entrada);
    cmd
}

/// Transcribe el audio con tiempos por palabra alineados con la voz:
/// 1) detecta silencios, 2) los quita y transcribe solo la voz (whisper es mucho
/// más preciso así), 3) devuelve cada palabra a su tiempo real en el clip.
#[allow(clippy::too_many_arguments)]
fn transcribir(
    compartido: &Arc<Mutex<Compartido>>,
    idx: usize,
    entrada: &Path,
    inicio: Option<f64>,
    largo: Option<f64>,
    duracion: f64,
    config: &Config,
    base: &Path,
    json_rel: &str,
    ctx: &egui::Context,
) -> Result<Vec<st::Palabra>, String> {
    let s = &config.subs;
    if !s.modelo.instalado() {
        return Err("Falta descargar el modelo de subtítulos".into());
    }
    let _ = std::fs::create_dir_all(base.join("tmp"));
    let json_abs = base.join(json_rel);
    let _ = std::fs::remove_file(&json_abs);

    // 1) Detectar silencios (solo audio, muy rápido)
    if let Some(t) = compartido.lock().unwrap().trabajos.get_mut(idx) {
        t.estado = Estado::Procesando("Analizando la voz", 0.0);
    }
    ctx.request_repaint();
    let mut cmd = ffmpeg_con_rango(entrada, inicio, largo, None);
    cmd.args([
        "-vn",
        "-map",
        "0:a:0",
        "-af",
        "silencedetect=noise=-35dB:d=0.35",
        "-f",
        "null",
        "-",
    ])
    .args(["-nostats", "-loglevel", "info"])
    .stdout(Stdio::null())
    .stderr(Stdio::piped());
    let log = cmd
        .output()
        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
        .unwrap_or_default();
    let tramos = st::tramos_con_voz(&log, duracion);
    let voz: f64 = tramos.iter().map(|(a, b)| b - a).sum();
    if tramos.is_empty() {
        return Ok(Vec::new());
    }
    // Si casi no hay silencios (p. ej. música de fondo), se transcribe todo tal cual.
    let recortar = tramos.len() > 1 || voz < duracion - 0.3;

    // 2) Transcribir solo la voz, con tiempos por palabra (max_len=1)
    let previo = if recortar {
        format!(
            "aresample=16000,asetnsamples=n=160:p=0,aselect='{}',asetpts=N/SR/TB,",
            st::expresion_aselect(&tramos)
        )
    } else {
        String::new()
    };
    let filtro = format!(
        "{previo}whisper=model=models/{}:language={}:queue=20:format=json:destination={json_rel}:max_len=1",
        s.modelo.archivo(),
        s.idioma.codigo()
    );
    let mut cmd = ffmpeg_con_rango(entrada, inicio, largo, None);
    cmd.current_dir(base)
        .args(["-vn", "-map", "0:a:0", "-af", &filtro, "-f", "null", "-"]);
    let r = ejecutar_ffmpeg(
        compartido,
        idx,
        cmd,
        if recortar { voz } else { duracion },
        "Transcribiendo audio",
        ctx,
    );
    let json = std::fs::read_to_string(&json_abs).unwrap_or_default();
    let _ = std::fs::remove_file(&json_abs);
    r.map_err(|e| format!("No se pudo transcribir (¿el video tiene audio?): {e}"))?;

    // 3) Tiempos reales
    Ok(st::leer_palabras(
        &json,
        if recortar { &tramos } else { &[] },
    ))
}

fn convertir(
    compartido: &Arc<Mutex<Compartido>>,
    idx: usize,
    trabajo: &Trabajo,
    config: &Config,
    ctx: &egui::Context,
) -> Result<(PathBuf, String), String> {
    let entrada = &trabajo.entrada;
    let (inicio, largo) = trabajo.rango()?;
    let total = trabajo.duracion.unwrap_or(0.0);
    let duracion = largo.unwrap_or(total - inicio.unwrap_or(0.0));
    let solo_subs = config.que == QueGenerar::SoloSubtitulos;

    // Las rutas dentro de los filtros de ffmpeg no pueden llevar "C:", así que
    // ffmpeg se ejecuta dentro de la carpeta de la app y usa rutas relativas.
    let base = carpeta_app();
    let id = format!("{}_{idx}", std::process::id());
    let json_rel = format!("tmp/palabras_{id}.json");
    let ass_rel = format!("tmp/subs_{id}.ass");
    let ass_abs = base.join(&ass_rel);
    let mut palabras: Vec<st::Palabra> = Vec::new();

    if config.necesita_whisper() {
        palabras = transcribir(
            compartido, idx, entrada, inicio, largo, duracion, config, &base, &json_rel, ctx,
        )?;
        if compartido.lock().unwrap().cancelar {
            return Err("Cancelado".into());
        }
    }

    // ── Modo «solo subtítulos»: guardar el .srt y terminar ──
    if solo_subs {
        if palabras.is_empty() {
            return Err("No se detectó voz en el audio".into());
        }
        // Con recorte, las marcas se desplazan para que coincidan con el video original.
        let srt = st::generar_srt(
            &palabras,
            config.subs.aspecto.mayusculas,
            inicio.unwrap_or(0.0),
        );
        let salida = ruta_salida(trabajo, config, "", "srt");
        std::fs::write(&salida, srt).map_err(|e| format!("No se pudo guardar el .srt: {e}"))?;
        return Ok((salida, format!("{} palabras", palabras.len())));
    }

    // ── Modo video ──
    let salida = ruta_salida(trabajo, config, "_9x16", "mp4");
    let con_subs = config.subs.activar && !palabras.is_empty();
    if con_subs {
        st::instalar_fuentes(&base);
        std::fs::write(&ass_abs, st::generar_ass(&palabras, &config.subs.aspecto))
            .map_err(|e| e.to_string())?;
    }
    let filtro = construir_filtro(config, con_subs.then_some(ass_rel.as_str()));

    // GPU primero (si está activada y disponible); si falla, se reintenta con CPU.
    let mut candidatos: Vec<Codificador> = match config.aceleracion {
        Aceleracion::Automatica => gpus_disponibles().to_vec(),
        Aceleracion::SoloCpu => Vec::new(),
    };
    candidatos.push(Codificador::Cpu);

    let mut resultado = Err("Sin codificador".to_string());
    let mut usado = Codificador::Cpu;
    let mut fallos: Vec<&str> = Vec::new();
    for cod in candidatos {
        let mut cmd = ffmpeg_con_rango(entrada, inicio, largo, cod.hwaccel());
        cmd.current_dir(&base)
            .args(["-filter_complex", &filtro])
            .args(["-map", "[v]", "-map", "0:a:0?"])
            .args(cod.args(config.calidad_crf))
            .args(["-c:a", "aac", "-b:a", "192k"])
            .args(["-movflags", "+faststart"])
            .arg(&salida);
        let etapa = match (con_subs, cod == Codificador::Cpu) {
            (true, false) => "Generando video con subtítulos (GPU)",
            (true, true) => "Generando video con subtítulos (CPU)",
            (false, false) => "Generando video (GPU)",
            (false, true) => "Generando video (CPU)",
        };
        resultado = ejecutar_ffmpeg(compartido, idx, cmd, duracion, etapa, ctx);
        usado = cod;
        if resultado.is_ok() || compartido.lock().unwrap().cancelar {
            break;
        }
        let _ = std::fs::remove_file(&salida);
        fallos.push(cod.nombre());
    }
    let nota = if fallos.is_empty() {
        usado.nombre().to_string()
    } else {
        format!("{}; falló: {}", usado.nombre(), fallos.join(", "))
    };

    if con_subs && config.subs.guardar_srt && resultado.is_ok() {
        let srt = st::generar_srt(&palabras, config.subs.aspecto.mayusculas, 0.0);
        let _ = std::fs::write(salida.with_extension("srt"), srt);
    }
    let _ = std::fs::remove_file(&ass_abs);

    match resultado {
        Ok(()) => Ok((salida, nota)),
        Err(e) => {
            let _ = std::fs::remove_file(&salida);
            Err(e)
        }
    }
}

// ───────────────────────────── Interfaz ─────────────────────────────

impl App {
    fn panel_opciones(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(8.0);
        ui.strong("¿Qué quieres generar?");
        ui.radio_value(&mut self.config.que, QueGenerar::Video, "🎬 Video 9:16");
        ui.radio_value(
            &mut self.config.que,
            QueGenerar::SoloSubtitulos,
            "📝 Solo subtítulos (.srt)",
        );
        let solo_subs = self.config.que == QueGenerar::SoloSubtitulos;

        if !solo_subs {
            ui.separator();
            ui.strong("Modo de ajuste");
            for m in [Modo::FondoDesenfocado, Modo::Recortar, Modo::BarrasNegras] {
                ui.radio_value(&mut self.config.modo, m, m.nombre());
            }
            if self.config.modo == Modo::Recortar {
                ui.add_space(4.0);
                ui.label("Posición del recorte:");
                ui.add(egui::Slider::new(&mut self.config.posicion_x, 0.0..=1.0).text("izq ↔ der"));
            }
        }

        ui.separator();
        self.panel_subtitulos(ui, ctx, solo_subs);

        if !solo_subs {
            ui.separator();
            ui.strong("Resolución");
            ui.radio_value(
                &mut self.config.resolucion,
                Resolucion::R1080,
                "1080 × 1920 (Full HD)",
            );
            ui.radio_value(
                &mut self.config.resolucion,
                Resolucion::R720,
                "720 × 1280 (HD)",
            );

            ui.separator();
            ui.strong("Calidad");
            ui.add(egui::Slider::new(&mut self.config.calidad_crf, 16..=30).text("CRF"));
            ui.small("Menor CRF = mejor calidad y archivo más grande.");

            ui.separator();
            ui.strong("Aceleración");
            ui.radio_value(
                &mut self.config.aceleracion,
                Aceleracion::Automatica,
                "⚡ GPU (automático, CPU si falla)",
            );
            ui.radio_value(
                &mut self.config.aceleracion,
                Aceleracion::SoloCpu,
                "Solo CPU",
            );
            match GPUS.get() {
                None => {
                    ui.small("Detectando GPU…");
                }
                Some(g) if g.is_empty() => {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "No se encontró GPU compatible: se usará la CPU.",
                    );
                }
                Some(g) => {
                    ui.small(format!(
                        "Disponible: {}",
                        g.iter().map(|c| c.nombre()).collect::<Vec<_>>().join(", ")
                    ));
                }
            }
        }

        ui.separator();
        ui.strong("Carpeta de salida");
        match &self.config.carpeta_salida {
            Some(p) => ui.label(p.display().to_string()),
            None => ui.label("La misma carpeta del video"),
        };
        ui.horizontal(|ui| {
            if ui.button("🗁 Elegir…").clicked() {
                if let Some(d) = rfd::FileDialog::new().pick_folder() {
                    self.config.carpeta_salida = Some(d);
                }
            }
            if self.config.carpeta_salida.is_some() && ui.button("Restablecer").clicked() {
                self.config.carpeta_salida = None;
            }
        });
        ui.add_space(8.0);
    }

    fn panel_subtitulos(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, solo_subs: bool) {
        ui.strong(if solo_subs {
            "Transcripción"
        } else {
            "Subtítulos automáticos"
        });
        if !self.whisper_ok {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                "Tu ffmpeg no incluye whisper. Instala la versión «full» de gyan.dev.",
            );
            return;
        }
        if !solo_subs {
            ui.checkbox(&mut self.config.subs.activar, "Agregar subtítulos al video");
            if !self.config.subs.activar {
                return;
            }
        }
        let s = &mut self.config.subs;

        egui::ComboBox::from_label("Idioma")
            .selected_text(s.idioma.nombre())
            .show_ui(ui, |ui| {
                for i in [Idioma::Es, Idioma::En, Idioma::Pt, Idioma::Auto] {
                    ui.selectable_value(&mut s.idioma, i, i.nombre());
                }
            });

        egui::ComboBox::from_label("Precisión")
            .selected_text(s.modelo.nombre())
            .show_ui(ui, |ui| {
                for m in [Modelo::Base, Modelo::Small, Modelo::Turbo] {
                    let marca = if m.instalado() { "✔ " } else { "" };
                    ui.selectable_value(&mut s.modelo, m, format!("{marca}{}", m.nombre()));
                }
            });

        let descarga = self.descarga.lock().unwrap().clone();
        let modelo = s.modelo;
        match descarga {
            Descarga::Descargando(m, p) => {
                ui.label(format!("Descargando modelo {}…", m.nombre()));
                ui.add(egui::ProgressBar::new(p).show_percentage());
            }
            _ if !modelo.instalado() => {
                ui.colored_label(egui::Color32::YELLOW, "Este modelo aún no está descargado.");
                if ui
                    .button(format!(
                        "⬇ Descargar ({} MB)",
                        modelo.bytes_aprox() / 1_000_000
                    ))
                    .clicked()
                {
                    self.descargar_modelo(modelo, ctx.clone());
                }
                if let Descarga::Error(e) = descarga {
                    ui.colored_label(egui::Color32::LIGHT_RED, e);
                }
            }
            _ => {}
        }

        let a = &mut self.config.subs.aspecto;
        if !solo_subs {
            ui.add_space(4.0);
            egui::ComboBox::from_label("Estilo")
                .selected_text(a.estilo.nombre())
                .show_ui(ui, |ui| {
                    for e in [
                        st::Estilo::Resaltado,
                        st::Estilo::PalabraPorPalabra,
                        st::Estilo::Frase,
                    ] {
                        ui.selectable_value(&mut a.estilo, e, e.nombre());
                    }
                });
            egui::ComboBox::from_label("Fuente")
                .selected_text(a.fuente.nombre())
                .show_ui(ui, |ui| {
                    for f in st::Fuente::TODAS {
                        ui.selectable_value(&mut a.fuente, f, f.nombre());
                    }
                });
            egui::ComboBox::from_label("Animación")
                .selected_text(a.animacion.nombre())
                .show_ui(ui, |ui| {
                    for an in [
                        st::Animacion::Pop,
                        st::Animacion::Deslizar,
                        st::Animacion::Fundido,
                        st::Animacion::Ninguna,
                    ] {
                        ui.selectable_value(&mut a.animacion, an, an.nombre());
                    }
                });
            if a.estilo != st::Estilo::Frase {
                ui.horizontal(|ui| {
                    ui.label("Resaltado:");
                    for c in st::ColorResaltado::TODOS {
                        let [r, g, b] = c.rgb();
                        let texto =
                            egui::RichText::new("⬤").color(egui::Color32::from_rgb(r, g, b));
                        ui.selectable_value(&mut a.resaltado, c, texto)
                            .on_hover_text(c.nombre());
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label("Tamaño:");
                ui.selectable_value(&mut a.tamano, st::Tamano::Pequeno, "S");
                ui.selectable_value(&mut a.tamano, st::Tamano::Mediano, "M");
                ui.selectable_value(&mut a.tamano, st::Tamano::Grande, "L");
            });
            ui.horizontal(|ui| {
                ui.label("Posición:");
                ui.selectable_value(&mut a.posicion, st::Posicion::Arriba, "Arriba");
                ui.selectable_value(&mut a.posicion, st::Posicion::Centro, "Centro");
                ui.selectable_value(&mut a.posicion, st::Posicion::Abajo, "Abajo");
            });
            ui.checkbox(&mut a.quitar_puntuacion, "Quitar comas y puntos");
        }
        ui.checkbox(&mut a.mayusculas, "TEXTO EN MAYÚSCULAS");
        if solo_subs {
            ui.small("Se guarda «nombre.srt» junto al video (o en la carpeta de salida).");
        } else {
            ui.checkbox(
                &mut self.config.subs.guardar_srt,
                "Guardar también el archivo .srt",
            );
            self.vista_previa(ui, ctx);
        }
    }

    /// Renderiza (en segundo plano) un fotograma de ejemplo con el estilo elegido.
    fn vista_previa(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let a = &self.config.subs.aspecto;
        let clave = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            a.estilo as u8,
            a.fuente as u8,
            a.animacion as u8,
            a.resaltado as u8,
            a.tamano as u8,
            a.posicion as u8,
            a.mayusculas,
            a.quitar_puntuacion
        );
        if clave != self.previa_clave && !self.previa_ocupada.swap(true, Ordering::SeqCst) {
            self.previa_clave = clave;
            let (aspecto, imagen, ocupada, ctx) = (
                a.clone(),
                Arc::clone(&self.previa_imagen),
                Arc::clone(&self.previa_ocupada),
                ctx.clone(),
            );
            thread::spawn(move || {
                let base = carpeta_app();
                st::instalar_fuentes(&base);
                let _ = std::fs::create_dir_all(base.join("tmp"));
                let ass = st::generar_ass(&st::palabras_de_ejemplo(), &aspecto);
                let _ = std::fs::write(base.join("tmp/previa.ass"), ass);
                let jpg = base.join("tmp/previa.jpg");
                let _ = std::fs::remove_file(&jpg);
                let _ = comando("ffmpeg")
                    .current_dir(&base)
                    .args([
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-y",
                        "-f",
                        "lavfi",
                        "-i",
                    ])
                    .arg("gradients=s=1080x1920:c0=0x3a6186:c1=0x89253e:d=2:speed=0")
                    .args([
                        "-vf",
                        "ass=filename=tmp/previa.ass:fontsdir=fonts,scale=360:640",
                    ])
                    .args(["-ss", "0.9", "-frames:v", "1", "-q:v", "3"])
                    .arg(&jpg)
                    .output();
                if let Ok(img) = image::open(&jpg) {
                    let img = img.to_rgba8();
                    let tam = [img.width() as usize, img.height() as usize];
                    *imagen.lock().unwrap() =
                        Some(egui::ColorImage::from_rgba_unmultiplied(tam, img.as_raw()));
                }
                ocupada.store(false, Ordering::SeqCst);
                ctx.request_repaint();
            });
        }
        if let Some(img) = self.previa_imagen.lock().unwrap().take() {
            self.previa_textura =
                Some(ctx.load_texture("previa_subs", img, egui::TextureOptions::LINEAR));
        }
        ui.add_space(4.0);
        ui.small("Vista previa:");
        match &self.previa_textura {
            Some(t) => {
                ui.add(egui::Image::new(egui::load::SizedTexture::new(
                    t.id(),
                    egui::vec2(135.0, 240.0),
                )));
            }
            None => {
                ui.small("generando…");
            }
        }
    }

    fn vista_carpeta(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, en_curso: bool) {
        let mut abrir: Option<PathBuf> = None;
        let mut agregar: Vec<PathBuf> = Vec::new();

        ui.horizontal(|ui| {
            if ui.button("🗁 Abrir carpeta…").clicked() {
                let mut d = rfd::FileDialog::new();
                if let Some(c) = &self.carpeta {
                    d = d.set_directory(c);
                }
                if let Some(dir) = d.pick_folder() {
                    abrir = Some(dir);
                }
            }
            if let Some(c) = &self.carpeta {
                if let Some(padre) = c.parent() {
                    if ui
                        .button("⬆ Subir")
                        .on_hover_text("Carpeta superior")
                        .clicked()
                    {
                        abrir = Some(padre.to_path_buf());
                    }
                }
                if ui.button("🔄").on_hover_text("Actualizar").clicked() {
                    abrir = Some(c.clone());
                }
                ui.add(
                    egui::Label::new(egui::RichText::new(c.display().to_string()).weak())
                        .truncate(),
                )
                .on_hover_text(c.display().to_string());
            }
        });

        let Some(_) = &self.carpeta else {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label("Abre una carpeta para ver aquí todos sus videos.");
            });
            if let Some(d) = abrir {
                self.abrir_carpeta(d, ctx);
            }
            return;
        };

        // Subcarpetas
        if !self.subcarpetas.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for sc in &self.subcarpetas {
                    let n = sc
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if ui.button(format!("🗀 {n}")).clicked() {
                        abrir = Some(sc.clone());
                    }
                }
            });
        }

        let ya_en_cola = self.en_cola();
        let filtro = self.busqueda.to_lowercase();
        let visibles: Vec<usize> = (0..self.videos.len())
            .filter(|&i| {
                filtro.is_empty() || self.videos[i].nombre.to_lowercase().contains(&filtro)
            })
            .collect();

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.add(
                egui::TextEdit::singleline(&mut self.busqueda)
                    .hint_text("Buscar…")
                    .desired_width(160.0),
            );
            ui.label(format!("{} video(s)", visibles.len()));
            ui.add_enabled_ui(!en_curso, |ui| {
                let sel: Vec<PathBuf> = self
                    .seleccion
                    .iter()
                    .filter(|p| !ya_en_cola.contains(*p))
                    .cloned()
                    .collect();
                if ui
                    .add_enabled(
                        !sel.is_empty(),
                        egui::Button::new(format!("➕ Agregar seleccionados ({})", sel.len())),
                    )
                    .clicked()
                {
                    agregar.extend(sel);
                }
                if ui.button("➕ Agregar todos").clicked() {
                    agregar.extend(visibles.iter().map(|&i| self.videos[i].ruta.clone()));
                }
            });
        });
        if let Some(a) = &self.aviso {
            ui.colored_label(egui::Color32::LIGHT_GREEN, a);
        }
        ui.separator();

        if self.videos.is_empty() {
            ui.label("No hay videos en esta carpeta.");
        }

        // Cargar en la GPU las miniaturas que ya estén listas
        {
            let mut info = self.info.lock().unwrap();
            for (ruta, i) in info.iter_mut() {
                if let Some(img) = i.imagen.take() {
                    let tex =
                        ctx.load_texture(ruta.to_string_lossy(), img, egui::TextureOptions::LINEAR);
                    self.texturas.insert(ruta.clone(), tex);
                }
            }
        }

        let tam_mini = egui::vec2(176.0, 99.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let ancho_tarjeta = tam_mini.x + 24.0;
                let columnas = ((ui.available_width() + 10.0) / (ancho_tarjeta + 10.0))
                    .floor()
                    .max(1.0) as usize;
                let info = self.info.lock().unwrap();
                for fila in visibles.chunks(columnas) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
                        for &i in fila {
                            let v = &self.videos[i];
                            let datos = info.get(&v.ruta);
                            let en_la_cola = ya_en_cola.contains(&v.ruta);
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.set_width(tam_mini.x);
                                ui.vertical(|ui| {
                                    // Miniatura (clic = reproducir)
                                    let resp = match self.texturas.get(&v.ruta) {
                                        Some(tex) => ui.add(
                                            egui::Image::new(egui::load::SizedTexture::new(
                                                tex.id(),
                                                tam_mini,
                                            ))
                                            .sense(egui::Sense::click()),
                                        ),
                                        None => {
                                            let (r, p) =
                                                ui.allocate_painter(tam_mini, egui::Sense::click());
                                            p.rect_filled(
                                                r.rect,
                                                4.0,
                                                egui::Color32::from_gray(40),
                                            );
                                            let txt = if datos.is_some() {
                                                "sin vista previa"
                                            } else {
                                                "⏳ cargando…"
                                            };
                                            p.text(
                                                r.rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                txt,
                                                egui::FontId::proportional(13.0),
                                                egui::Color32::GRAY,
                                            );
                                            r
                                        }
                                    };
                                    if resp.on_hover_text("Clic para reproducir").clicked() {
                                        abrir_con_sistema(&v.ruta);
                                    }

                                    ui.add(
                                        egui::Label::new(egui::RichText::new(&v.nombre).strong())
                                            .truncate(),
                                    )
                                    .on_hover_text(&v.nombre);
                                    let mut detalle = Vec::new();
                                    if let Some(d) = datos.and_then(|d| d.duracion) {
                                        detalle.push(formatear_tiempo(d));
                                    }
                                    if let Some((w, h)) = datos.and_then(|d| d.dims) {
                                        detalle.push(format!("{w}×{h}"));
                                    }
                                    detalle.push(formatear_bytes(v.bytes));
                                    ui.small(detalle.join(" · "));

                                    ui.horizontal(|ui| {
                                        if en_la_cola {
                                            ui.colored_label(
                                                egui::Color32::LIGHT_GREEN,
                                                "✔ En la cola",
                                            );
                                        } else {
                                            let mut marcado = self.seleccion.contains(&v.ruta);
                                            if ui
                                                .checkbox(&mut marcado, "")
                                                .on_hover_text("Seleccionar")
                                                .changed()
                                            {
                                                if marcado {
                                                    self.seleccion.insert(v.ruta.clone());
                                                } else {
                                                    self.seleccion.remove(&v.ruta);
                                                }
                                            }
                                            if ui
                                                .add_enabled(
                                                    !en_curso,
                                                    egui::Button::new("➕ Agregar"),
                                                )
                                                .clicked()
                                            {
                                                agregar.push(v.ruta.clone());
                                            }
                                        }
                                        if ui
                                            .small_button("🗁")
                                            .on_hover_text("Mostrar en el Explorador")
                                            .clicked()
                                        {
                                            mostrar_en_explorador(&v.ruta);
                                        }
                                    });
                                });
                            });
                        }
                    });
                    ui.add_space(6.0);
                }
            });

        if !agregar.is_empty() {
            for p in &agregar {
                self.seleccion.remove(p);
            }
            self.agregar_a_cola(agregar);
        }
        if let Some(d) = abrir {
            self.abrir_carpeta(d, ctx);
        }
    }

    fn vista_cola(&mut self, ui: &mut egui::Ui, en_curso: bool) {
        let solo_subs = self.config.que == QueGenerar::SoloSubtitulos;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!en_curso, |ui| {
                if ui.button("➕ Seleccionar videos…").clicked() {
                    if let Some(archivos) = rfd::FileDialog::new()
                        .add_filter("Videos", EXTENSIONES)
                        .add_filter("Todos", &["*"])
                        .pick_files()
                    {
                        self.agregar_a_cola(archivos);
                    }
                }
                if ui.button("🗑 Limpiar lista").clicked() {
                    self.compartido.lock().unwrap().trabajos.clear();
                }
            });
        });
        ui.small("Inicio/Fin vacíos = video completo (formato 90, 1:30 o 1:02:03).");
        ui.separator();

        let mut trabajos = self.compartido.lock().unwrap().trabajos.clone();
        if trabajos.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("La cola está vacía. Agrega videos desde «Explorar carpeta», con «Seleccionar videos…» o arrastrándolos aquí.");
            });
            return;
        }

        let mut quitar = None;
        let mut duplicar = None;
        let mut editado: Vec<usize> = Vec::new();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (i, t) in trabajos.iter_mut().enumerate() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let nombre =
                            t.entrada.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.strong(nombre);
                        if let Some(d) = t.duracion {
                            ui.weak(format!("({})", formatear_tiempo(d)));
                        }
                        if !en_curso {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("✖").on_hover_text("Quitar").clicked() {
                                    quitar = Some(i);
                                }
                                if ui
                                    .small_button("⧉ Otro clip")
                                    .on_hover_text("Duplicar para sacar otro clip de este video")
                                    .clicked()
                                {
                                    duplicar = Some(i);
                                }
                            });
                        }
                    });
                    ui.add_enabled_ui(!en_curso, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("✂ Inicio:");
                            let r1 = ui.add(
                                egui::TextEdit::singleline(&mut t.inicio).hint_text("0:00").desired_width(80.0),
                            );
                            ui.label("Fin:");
                            let pista = t.duracion.map(formatear_tiempo).unwrap_or("final".into());
                            let r2 =
                                ui.add(egui::TextEdit::singleline(&mut t.fin).hint_text(pista).desired_width(80.0));
                            if r1.changed() || r2.changed() {
                                editado.push(i);
                            }
                            match t.rango() {
                                Ok((ini, largo)) => {
                                    let l = largo.or(t.duracion.map(|d| d - ini.unwrap_or(0.0)));
                                    if let Some(l) = l {
                                        let txt = format!("Duración del clip: {}", formatear_tiempo(l));
                                        if l > 60.0 && !solo_subs {
                                            ui.colored_label(egui::Color32::YELLOW, txt).on_hover_text(
                                                "Más de 60 s: YouTube Shorts permite hasta 3 min; otras plataformas varían",
                                            );
                                        } else {
                                            ui.weak(txt);
                                        }
                                    }
                                }
                                Err(e) => {
                                    ui.colored_label(egui::Color32::LIGHT_RED, format!("⚠ {e}"));
                                }
                            }
                        });
                    });
                    match &t.estado {
                        Estado::Pendiente => {
                            ui.label("En espera");
                        }
                        Estado::Procesando(etapa, p) => {
                            ui.add(
                                egui::ProgressBar::new(*p).text(format!("{etapa}… {:.0}%", p * 100.0)).animate(true),
                            );
                        }
                        Estado::Listo(salida, nota) => {
                            ui.horizontal(|ui| {
                                ui.colored_label(egui::Color32::LIGHT_GREEN, "✔ Listo");
                                if !nota.is_empty() {
                                    ui.weak(format!("({nota})"));
                                }
                                if ui.link(salida.display().to_string()).clicked() {
                                    mostrar_en_explorador(salida);
                                }
                            });
                        }
                        Estado::Error(e) => {
                            ui.colored_label(egui::Color32::LIGHT_RED, format!("✖ Error: {e}"));
                        }
                        Estado::Cancelado => {
                            ui.colored_label(egui::Color32::YELLOW, "Cancelado");
                        }
                    }
                });
            }
        });

        let mut c = self.compartido.lock().unwrap();
        for i in editado {
            if let (Some(dst), Some(src)) = (c.trabajos.get_mut(i), trabajos.get(i)) {
                dst.inicio = src.inicio.clone();
                dst.fin = src.fin.clone();
                // Cambió el rango: se puede volver a generar
                dst.estado = Estado::Pendiente;
            }
        }
        if let Some(i) = duplicar {
            if let Some(t) = c.trabajos.get(i).cloned() {
                c.trabajos.insert(
                    i + 1,
                    Trabajo {
                        estado: Estado::Pendiente,
                        inicio: t.fin.clone(),
                        fin: String::new(),
                        ..t
                    },
                );
            }
        }
        if let Some(i) = quitar {
            c.trabajos.remove(i);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Arrastrar y soltar archivos o carpetas
        let soltados: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !soltados.is_empty() {
            if let Some(dir) = soltados.iter().find(|p| p.is_dir()) {
                self.abrir_carpeta(dir.clone(), ctx);
                self.pestana = Pestana::Carpeta;
            }
            let archivos: Vec<PathBuf> = soltados.into_iter().filter(|p| p.is_file()).collect();
            if !archivos.is_empty() {
                self.agregar_a_cola(archivos);
                self.pestana = Pestana::Cola;
            }
        }

        let en_curso = self.compartido.lock().unwrap().en_curso;

        egui::TopBottomPanel::top("titulo").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.heading("🎬 Formato Shorts 9:16");
            ui.label("Convierte videos a vertical para Shorts, Reels y TikTok, con subtítulos automáticos.");
            ui.add_space(6.0);
        });

        egui::SidePanel::left("opciones")
            .min_width(270.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_enabled_ui(!en_curso, |ui| self.panel_opciones(ui, ctx));
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if !self.ffmpeg_ok {
                ui.colored_label(
                    egui::Color32::LIGHT_RED,
                    "⚠ No se encontró ffmpeg/ffprobe en el PATH. Instálalo con: winget install Gyan.FFmpeg",
                );
                ui.separator();
            }

            let (n_cola, hay_pendientes, hay_errores) = {
                let c = self.compartido.lock().unwrap();
                let pend: Vec<&Trabajo> =
                    c.trabajos.iter().filter(|t| !matches!(t.estado, Estado::Listo(..))).collect();
                (c.trabajos.len(), !pend.is_empty(), pend.iter().any(|t| t.rango().is_err()))
            };
            let falta_modelo = self.config.necesita_whisper() && !self.config.subs.modelo.instalado();
            let solo_subs = self.config.que == QueGenerar::SoloSubtitulos;

            // Pestañas + botón principal
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.pestana, Pestana::Carpeta, "🗁 Explorar carpeta");
                ui.selectable_value(&mut self.pestana, Pestana::Cola, format!("☰ Cola ({n_cola})"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if en_curso {
                        if ui.button("⏹ Cancelar").clicked() {
                            let mut c = self.compartido.lock().unwrap();
                            c.cancelar = true;
                            if let Some(h) = c.hijo.as_mut() {
                                let _ = h.kill();
                            }
                        }
                    } else {
                        let texto = if solo_subs { "▶ Generar subtítulos" } else { "▶ Generar 9:16" };
                        let r = ui.add_enabled(
                            hay_pendientes && !hay_errores && !falta_modelo && self.ffmpeg_ok,
                            egui::Button::new(egui::RichText::new(texto).strong()),
                        );
                        let r = if falta_modelo {
                            r.on_disabled_hover_text("Primero descarga el modelo de subtítulos (panel izquierdo)")
                        } else if !hay_pendientes {
                            r.on_disabled_hover_text("Agrega videos a la cola")
                        } else {
                            r
                        };
                        if r.clicked() {
                            self.pestana = Pestana::Cola;
                            self.iniciar(ctx.clone());
                        }
                    }
                });
            });
            ui.separator();

            match self.pestana {
                Pestana::Carpeta => self.vista_carpeta(ui, ctx, en_curso),
                Pestana::Cola => self.vista_cola(ui, en_curso),
            }
        });
    }
}

/// Configuración inicial (la usa la interfaz y el modo --cli).
fn config_por_defecto(modelo: Modelo) -> Config {
    Config {
        que: QueGenerar::Video,
        modo: Modo::FondoDesenfocado,
        resolucion: Resolucion::R1080,
        posicion_x: 0.5,
        calidad_crf: 20,
        aceleracion: Aceleracion::Automatica,
        carpeta_salida: None,
        subs: OpcionesSubs {
            activar: false,
            modelo,
            idioma: Idioma::Es,
            aspecto: st::Opciones {
                estilo: st::Estilo::Resaltado,
                fuente: st::Fuente::Luckiest,
                animacion: st::Animacion::Pop,
                resaltado: st::ColorResaltado::Amarillo,
                tamano: st::Tamano::Mediano,
                posicion: st::Posicion::Abajo,
                mayusculas: true,
                quitar_puntuacion: true,
            },
            guardar_srt: true,
        },
    }
}

/// Mejor modelo de whisper ya descargado (o el básico).
fn mejor_modelo() -> Modelo {
    [Modelo::Turbo, Modelo::Small, Modelo::Base]
        .into_iter()
        .find(|m| m.instalado())
        .unwrap_or(Modelo::Base)
}

const USO_CLI: &str = "Uso: FormatoShorts.exe --cli <video> [opciones]

  --clip INICIO-FIN   Un clip (se puede repetir para sacar varios). Ej: --clip 0:15-0:45
  --inicio T --fin T  Un solo clip (forma antigua)
  --salida CARPETA    Dónde guardar (por defecto: <carpeta del video>/<nombre>_clips)
  --sin-subs          Sin subtítulos
  --solo-srt          Solo genera los .srt
  --cpu               No usar la GPU

Ejemplo:
  FormatoShorts.exe --cli video.mp4 --clip 0:15-0:45 --clip 1:30-2:10 --clip 5:00-5:40";

/// Modo línea de comandos: genera uno o varios clips 9:16 de un mismo video.
fn modo_cli(args: &[String]) -> i32 {
    let Some(entrada) = args.first().filter(|a| !a.starts_with("--")) else {
        eprintln!("{USO_CLI}");
        return 2;
    };
    // ffmpeg se ejecuta desde la carpeta de la app: las rutas deben ser absolutas.
    let absoluta = |p: PathBuf| std::path::absolute(&p).unwrap_or(p);
    let entrada = absoluta(PathBuf::from(entrada));
    if !entrada.is_file() {
        eprintln!("ERROR no existe el video: {}", entrada.display());
        return 2;
    }
    let mut config = config_por_defecto(mejor_modelo());
    config.subs.activar = true;
    let mut clips: Vec<(String, String)> = Vec::new();
    let (mut inicio, mut fin) = (String::new(), String::new());
    let mut salida: Option<PathBuf> = None;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        let mut valor = |nombre: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("Falta el valor de {nombre}"))
        };
        let r: Result<(), String> = (|| {
            match a.as_str() {
                "--clip" => {
                    let v = valor("--clip")?;
                    let (a, b) = v
                        .split_once('-')
                        .ok_or_else(|| format!("Clip no válido «{v}», usa INICIO-FIN (ej. 1:30-2:10)"))?;
                    clips.push((a.trim().to_string(), b.trim().to_string()));
                }
                "--inicio" => inicio = valor("--inicio")?,
                "--fin" => fin = valor("--fin")?,
                "--salida" => salida = Some(absoluta(PathBuf::from(valor("--salida")?))),
                "--sin-subs" => config.subs.activar = false,
                "--solo-srt" => config.que = QueGenerar::SoloSubtitulos,
                "--cpu" => config.aceleracion = Aceleracion::SoloCpu,
                "--ayuda" | "--help" | "-h" => return Err(String::new()),
                otro => return Err(format!("Opción desconocida: {otro}")),
            }
            Ok(())
        })();
        if let Err(e) = r {
            if !e.is_empty() {
                eprintln!("ERROR {e}\n");
            }
            eprintln!("{USO_CLI}");
            return 2;
        }
    }
    let varios = !clips.is_empty();
    if !varios {
        clips.push((inicio, fin));
    }

    let duracion = probar_video(&entrada).0;
    let base = entrada
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "video".into());
    // Con varios clips se guardan en una carpeta propia: <video>_clips
    let carpeta = salida.or_else(|| {
        varios.then(|| {
            entrada
                .parent()
                .unwrap_or(Path::new("."))
                .join(format!("{base}_clips"))
        })
    });
    if let Some(c) = &carpeta {
        if let Err(e) = std::fs::create_dir_all(c) {
            eprintln!("ERROR no se pudo crear la carpeta {}: {e}", c.display());
            return 1;
        }
        config.carpeta_salida = Some(c.clone());
    }

    let trabajos: Vec<Trabajo> = clips
        .iter()
        .enumerate()
        .map(|(n, (ini, fin))| {
            let mut t = Trabajo::con_duracion(entrada.clone(), duracion);
            t.inicio = ini.clone();
            t.fin = fin.clone();
            if varios {
                t.nombre = Some(format!("{base}_clip{:02}", n + 1));
            }
            t
        })
        .collect();
    let compartido = Arc::new(Mutex::new(Compartido {
        trabajos: trabajos.clone(),
        ..Default::default()
    }));
    let ctx = egui::Context::default();
    let total = trabajos.len();
    let mut fallidos = 0;
    for (i, t) in trabajos.iter().enumerate() {
        let rango = format!(
            "{}-{}",
            if t.inicio.is_empty() { "inicio" } else { &t.inicio },
            if t.fin.is_empty() { "fin" } else { &t.fin }
        );
        eprintln!("[{}/{total}] {rango} ...", i + 1);
        match convertir(&compartido, i, t, &config, &ctx) {
            Ok((ruta, nota)) => println!("OK {} ({nota})", ruta.display()),
            Err(e) => {
                fallidos += 1;
                println!("ERROR clip {} ({rango}): {e}", i + 1);
            }
        }
    }
    if let Some(c) = &carpeta {
        eprintln!("Listo: {} de {total} clips en {}", total - fallidos, c.display());
    }
    if fallidos > 0 { 1 } else { 0 }
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--cli") {
        // La versión final no tiene consola propia: se usa la de quien la ejecuta.
        #[cfg(windows)]
        {
            unsafe extern "system" {
                fn AttachConsole(id: u32) -> i32;
            }
            unsafe { AttachConsole(u32::MAX) };
        }
        std::process::exit(modo_cli(&args[1..]));
    }
    let opciones = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1060.0, 700.0])
            .with_min_inner_size([760.0, 480.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Formato Shorts 9:16",
        opciones,
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
    )
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn tiempos() {
        assert_eq!(parsear_tiempo(""), Ok(None));
        assert_eq!(parsear_tiempo("90"), Ok(Some(90.0)));
        assert_eq!(parsear_tiempo("1:30"), Ok(Some(90.0)));
        assert_eq!(parsear_tiempo("1:02:03,5"), Ok(Some(3723.5)));
        assert!(parsear_tiempo("1:75").is_err());
        assert!(parsear_tiempo("abc").is_err());
    }

    #[test]
    fn rango_del_clip() {
        let mut t = Trabajo::con_duracion(PathBuf::from("x.mp4"), Some(100.0));
        t.inicio = "10".into();
        t.fin = "40".into();
        assert_eq!(t.rango(), Ok((Some(10.0), Some(30.0))));
        t.fin = "5".into();
        assert!(t.rango().is_err());
        t.fin = "200".into();
        assert!(t.rango().is_err());
    }
}
