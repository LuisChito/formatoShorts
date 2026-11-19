//! Subtítulos estilo Shorts/Reels/TikTok:
//! - tiempos por palabra alineados con la voz (se quitan los silencios antes de
//!   transcribir y luego se devuelve cada palabra a su tiempo real),
//! - frases cortas que se cortan en las pausas,
//! - palabra activa resaltada, animaciones de entrada/salida y fuentes incluidas.

use std::path::PathBuf;

// ───────────────────────────── Opciones ─────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum Estilo {
    /// Frase corta con la palabra que se está diciendo resaltada.
    Resaltado,
    /// Una sola palabra grande a la vez.
    PalabraPorPalabra,
    /// Frase corta sin resaltado.
    Frase,
}

impl Estilo {
    pub fn nombre(self) -> &'static str {
        match self {
            Estilo::Resaltado => "Resaltar palabra (karaoke)",
            Estilo::PalabraPorPalabra => "Palabra por palabra",
            Estilo::Frase => "Frase simple",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Fuente {
    Poppins,
    Luckiest,
    Anton,
}

impl Fuente {
    pub const TODAS: [Fuente; 3] = [Fuente::Luckiest, Fuente::Poppins, Fuente::Anton];

    pub fn nombre(self) -> &'static str {
        match self {
            Fuente::Poppins => "Poppins (moderna)",
            Fuente::Luckiest => "Luckiest Guy (divertida)",
            Fuente::Anton => "Anton (impacto)",
        }
    }
    fn familia(self) -> &'static str {
        match self {
            Fuente::Poppins => "Poppins Black",
            Fuente::Luckiest => "Luckiest Guy",
            Fuente::Anton => "Anton",
        }
    }
    /// Tamaño base en píxeles para un video de 1920 px de alto.
    fn tamano_base(self) -> f64 {
        match self {
            Fuente::Poppins => 96.0,
            Fuente::Luckiest => 104.0,
            Fuente::Anton => 128.0,
        }
    }
}

/// Fuentes incluidas en el ejecutable (licencias OFL / Apache en assets/fonts).
const FUENTES: &[(&str, &[u8])] = &[
    (
        "Poppins-Black.ttf",
        include_bytes!("../assets/fonts/Poppins-Black.ttf"),
    ),
    (
        "LuckiestGuy-Regular.ttf",
        include_bytes!("../assets/fonts/LuckiestGuy-Regular.ttf"),
    ),
    (
        "Anton-Regular.ttf",
        include_bytes!("../assets/fonts/Anton-Regular.ttf"),
    ),
];

/// Copia las fuentes a <carpeta>/fonts si aún no están.
pub fn instalar_fuentes(carpeta_app: &std::path::Path) -> PathBuf {
    let dir = carpeta_app.join("fonts");
    let _ = std::fs::create_dir_all(&dir);
    for (nombre, datos) in FUENTES {
        let ruta = dir.join(nombre);
        let igual = std::fs::metadata(&ruta)
            .map(|m| m.len() == datos.len() as u64)
            .unwrap_or(false);
        if !igual {
            let _ = std::fs::write(&ruta, datos);
        }
    }
    dir
}

#[derive(Clone, Copy, PartialEq)]
pub enum Animacion {
    Pop,
    Deslizar,
    Fundido,
    Ninguna,
}

impl Animacion {
    pub fn nombre(self) -> &'static str {
        match self {
            Animacion::Pop => "Pop (rebote)",
            Animacion::Deslizar => "Deslizar hacia arriba",
            Animacion::Fundido => "Fundido",
            Animacion::Ninguna => "Sin animación",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum ColorResaltado {
    Amarillo,
    Verde,
    Cian,
    Rosa,
}

impl ColorResaltado {
    pub const TODOS: [ColorResaltado; 4] = [
        ColorResaltado::Amarillo,
        ColorResaltado::Verde,
        ColorResaltado::Cian,
        ColorResaltado::Rosa,
    ];

    pub fn nombre(self) -> &'static str {
        match self {
            ColorResaltado::Amarillo => "Amarillo",
            ColorResaltado::Verde => "Verde",
            ColorResaltado::Cian => "Cian",
            ColorResaltado::Rosa => "Rosa",
        }
    }
    /// Color RGB para mostrarlo en la interfaz.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            ColorResaltado::Amarillo => [0xFF, 0xE6, 0x00],
            ColorResaltado::Verde => [0x5C, 0xFF, 0x2B],
            ColorResaltado::Cian => [0x00, 0xE5, 0xFF],
            ColorResaltado::Rosa => [0xFF, 0x4F, 0xB8],
        }
    }
    /// Formato de color de ASS: &HBBGGRR&
    fn ass(self) -> String {
        let [r, g, b] = self.rgb();
        format!("&H{b:02X}{g:02X}{r:02X}&")
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tamano {
    Pequeno,
    Mediano,
    Grande,
}

impl Tamano {
    fn factor(self) -> f64 {
        match self {
            Tamano::Pequeno => 0.8,
            Tamano::Mediano => 1.0,
            Tamano::Grande => 1.25,
        }
    }
    /// (máx. palabras, máx. caracteres) por frase
    fn limites(self) -> (usize, usize) {
        match self {
            Tamano::Pequeno => (4, 26),
            Tamano::Mediano => (3, 18),
            Tamano::Grande => (2, 13),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Posicion {
    Arriba,
    Centro,
    Abajo,
}

impl Posicion {
    /// Altura (en un lienzo de 1920 px) del centro del texto.
    fn y(self) -> f64 {
        match self {
            Posicion::Arriba => 480.0,
            Posicion::Centro => 960.0,
            Posicion::Abajo => 1340.0, // por encima de los botones de Shorts/TikTok
        }
    }
}

#[derive(Clone)]
pub struct Opciones {
    pub estilo: Estilo,
    pub fuente: Fuente,
    pub animacion: Animacion,
    pub resaltado: ColorResaltado,
    pub tamano: Tamano,
    pub posicion: Posicion,
    pub mayusculas: bool,
    pub quitar_puntuacion: bool,
}

// ───────────────────────────── Palabras y tiempos ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Palabra {
    pub ini: f64,
    pub fin: f64,
    pub texto: String,
}

/// Paso de la rejilla (s) para recortar silencios: el audio se divide en bloques
/// de 10 ms, así el mapeo de tiempos es exacto.
const PASO: f64 = 0.01;

/// Analiza la salida de `silencedetect` y devuelve los tramos con voz,
/// con un pequeño margen y alineados a la rejilla de 10 ms.
pub fn tramos_con_voz(salida_silencedetect: &str, duracion: f64) -> Vec<(f64, f64)> {
    let mut silencios: Vec<(f64, f64)> = Vec::new();
    let mut inicio: Option<f64> = None;
    for l in salida_silencedetect.lines() {
        if let Some(p) = l.find("silence_start: ") {
            inicio = l[p + 15..]
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok());
        } else if let Some(p) = l.find("silence_end: ") {
            let fin: Option<f64> = l[p + 13..]
                .split_whitespace()
                .next()
                .and_then(|v| v.parse().ok());
            if let (Some(a), Some(b)) = (inicio.take(), fin) {
                silencios.push((a.max(0.0), b));
            }
        }
    }
    if let Some(a) = inicio {
        silencios.push((a.max(0.0), duracion));
    }

    // Complemento de los silencios = voz (con 0,15 s de margen a cada lado)
    let margen = 0.15;
    let mut voz: Vec<(f64, f64)> = Vec::new();
    let mut cursor = 0.0;
    for (a, b) in silencios {
        if a > cursor {
            voz.push(((cursor - margen).max(0.0), (a + margen).min(duracion)));
        }
        cursor = cursor.max(b);
    }
    if cursor < duracion {
        voz.push(((cursor - margen).max(0.0), duracion));
    }

    // Unir tramos que se tocan y alinearlos a la rejilla
    let mut unidos: Vec<(f64, f64)> = Vec::new();
    for (a, b) in voz {
        let (a, b) = ((a / PASO).round() * PASO, (b / PASO).round() * PASO);
        if b - a < PASO {
            continue;
        }
        match unidos.last_mut() {
            Some(u) if a <= u.1 + 0.05 => u.1 = u.1.max(b),
            _ => unidos.push((a, b)),
        }
    }
    unidos
}

/// Expresión de `aselect` que conserva solo los tramos con voz (bloques de 10 ms).
pub fn expresion_aselect(tramos: &[(f64, f64)]) -> String {
    tramos
        .iter()
        .map(|&(a, b)| {
            let k1 = (a / PASO).round();
            let k2 = (b / PASO).round() - 1.0;
            format!(
                "between(t\\,{:.3}\\,{:.3})",
                (k1 - 0.5) * PASO,
                (k2 + 0.5) * PASO
            )
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Convierte un tiempo del audio sin silencios al tiempo real del clip.
pub fn tiempo_real(t: f64, tramos: &[(f64, f64)]) -> f64 {
    let mut acumulado = 0.0;
    for &(a, b) in tramos {
        let largo = b - a;
        if t < acumulado + largo {
            return a + (t - acumulado);
        }
        acumulado += largo;
    }
    tramos.last().map(|t| t.1).unwrap_or(t)
}

/// Lee el JSON por líneas de whisper (`{"start":ms,"end":ms,"text":"..."}`),
/// descarta alucinaciones como «[Música]» y aplica el mapeo de tiempos.
pub fn leer_palabras(json: &str, tramos: &[(f64, f64)]) -> Vec<Palabra> {
    let mut palabras: Vec<Palabra> = Vec::new();
    for l in json.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l.trim()) else {
            continue;
        };
        let (Some(ini), Some(fin), Some(texto)) =
            (v["start"].as_f64(), v["end"].as_f64(), v["text"].as_str())
        else {
            continue;
        };
        let texto = texto.trim();
        if texto.is_empty()
            || texto.starts_with('[')
            || texto.starts_with('(')
            || texto.starts_with('*')
            || texto.contains('♪')
        {
            continue;
        }
        let (mut ini, mut fin) = (ini / 1000.0, fin / 1000.0);
        if !tramos.is_empty() {
            ini = tiempo_real(ini, tramos);
            fin = tiempo_real(fin, tramos);
            // Una palabra no cruza una pausa: si su fin «salta» al siguiente tramo
            // de voz, se corta al final del tramo donde empezó.
            if let Some(&(_, b)) = tramos.iter().find(|&&(a, b)| ini >= a && ini <= b) {
                fin = fin.min(b);
            }
        }
        // whisper a veces divide una palabra en dos trozos: si el trozo empieza
        // con un signo de puntuación pegado, se une a la anterior.
        if let Some(prev) = palabras.last_mut() {
            if texto.starts_with([',', '.', '!', '?', ';', ':']) {
                prev.texto.push_str(texto);
                prev.fin = fin.max(prev.fin);
                continue;
            }
        }
        palabras.push(Palabra {
            ini,
            fin: fin.max(ini + 0.05),
            texto: texto.to_string(),
        });
    }
    palabras
}

// ───────────────────────────── Frases ─────────────────────────────

fn termina_frase(texto: &str) -> bool {
    texto.ends_with(['.', '!', '?', '…'])
}

/// Agrupa palabras en frases cortas, cortando en pausas y finales de oración.
pub fn agrupar(
    palabras: &[Palabra],
    max_palabras: usize,
    max_chars: usize,
    pausa: f64,
) -> Vec<Vec<Palabra>> {
    let mut frases: Vec<Vec<Palabra>> = Vec::new();
    let mut actual: Vec<Palabra> = Vec::new();
    for p in palabras {
        if let Some(ult) = actual.last() {
            let largo: usize = actual
                .iter()
                .map(|w| w.texto.chars().count() + 1)
                .sum::<usize>()
                + p.texto.chars().count();
            let cortar = actual.len() >= max_palabras
                || largo > max_chars
                || p.ini - ult.fin > pausa
                || termina_frase(&ult.texto)
                || (ult.texto.ends_with(',') && actual.len() >= 2);
            if cortar {
                frases.push(std::mem::take(&mut actual));
            }
        }
        actual.push(p.clone());
    }
    if !actual.is_empty() {
        frases.push(actual);
    }
    frases
}

fn limpiar(texto: &str, op: &Opciones) -> String {
    let mut t = texto.to_string();
    if op.quitar_puntuacion {
        t = t.trim_matches(|c: char| ",.;:".contains(c)).to_string();
    }
    if op.mayusculas {
        t = t.to_uppercase();
    }
    // Caracteres especiales de ASS
    t.replace('\\', "").replace('{', "(").replace('}', ")")
}

fn marca_ass(seg: f64) -> String {
    let cs = (seg.max(0.0) * 100.0).round() as u64;
    format!(
        "{}:{:02}:{:02}.{:02}",
        cs / 360_000,
        (cs / 6000) % 60,
        (cs / 100) % 60,
        cs % 100
    )
}

pub fn marca_srt(seg: f64) -> String {
    let ms = (seg.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}

/// Fin de cada frase: un poco después de la última palabra, sin pisar la siguiente.
fn fin_de_frase(frases: &[Vec<Palabra>], i: usize, cola: f64) -> f64 {
    let ult = frases[i].last().unwrap().fin + cola;
    match frases.get(i + 1) {
        Some(sig) => ult.min(sig[0].ini).max(frases[i].last().unwrap().fin),
        None => ult,
    }
}

// ───────────────────────────── ASS (subtítulos quemados) ─────────────────────────────

/// Genera el archivo .ass con estilo, resaltado por palabra y animaciones.
pub fn generar_ass(palabras: &[Palabra], op: &Opciones) -> String {
    let palabra_a_palabra = op.estilo == Estilo::PalabraPorPalabra;
    let (max_p, max_c) = if palabra_a_palabra {
        (1, 40)
    } else {
        op.tamano.limites()
    };
    let frases = agrupar(palabras, max_p, max_c, 0.45);

    let factor = op.tamano.factor() * if palabra_a_palabra { 1.3 } else { 1.0 };
    let tamano = (op.fuente.tamano_base() * factor).round();
    let borde = (tamano * 0.075).round();
    let sombra = (tamano * 0.04).round();
    let y = op.posicion.y();
    let color = op.resaltado.ass();

    let mut s = format!(
        "[Script Info]\nScriptType: v4.00+\nPlayResX: 1080\nPlayResY: 1920\nWrapStyle: 0\nScaledBorderAndShadow: yes\n\n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Caption,{fam},{tamano},&H00FFFFFF,&H00FFFFFF,&H00000000,&H99000000,0,0,0,0,100,100,1,0,1,{borde},{sombra},5,80,80,0,1\n\n\
         [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        fam = op.fuente.familia()
    );

    for (i, frase) in frases.iter().enumerate() {
        let inicio = frase[0].ini;
        let fin = fin_de_frase(&frases, i, 0.25);
        let textos: Vec<String> = frase.iter().map(|p| limpiar(&p.texto, op)).collect();

        // Tramos dentro de la frase: uno por palabra activa (o uno solo si no hay resaltado)
        let mut tramos: Vec<(f64, f64, Option<usize>)> = Vec::new();
        if op.estilo == Estilo::Resaltado && frase.len() > 1 {
            for k in 0..frase.len() {
                let a = if k == 0 { inicio } else { frase[k].ini };
                let b = if k + 1 < frase.len() {
                    frase[k + 1].ini
                } else {
                    fin
                };
                if b > a {
                    tramos.push((a, b, Some(k)));
                }
            }
        } else {
            let k = if op.estilo == Estilo::Frase {
                None
            } else {
                Some(0)
            };
            tramos.push((inicio, fin, k));
        }

        let n = tramos.len();
        for (j, &(a, b, activa)) in tramos.iter().enumerate() {
            let primero = j == 0;
            let ultimo = j + 1 == n;
            let dur_ms = ((b - a) * 1000.0) as i64;

            let mut tags = String::new();
            match (op.animacion, primero) {
                (Animacion::Deslizar, true) => {
                    tags.push_str(&format!("\\move(540,{:.0},540,{y:.0},0,160)", y + 45.0));
                }
                _ => tags.push_str(&format!("\\pos(540,{y:.0})")),
            }
            if primero {
                match op.animacion {
                    Animacion::Pop => tags.push_str(
                        "\\fscx70\\fscy70\\t(0,90,\\fscx110\\fscy110)\\t(90,170,\\fscx100\\fscy100)",
                    ),
                    Animacion::Deslizar | Animacion::Fundido => {}
                    Animacion::Ninguna => {}
                }
            }
            let entrada =
                if primero && matches!(op.animacion, Animacion::Deslizar | Animacion::Fundido) {
                    160
                } else {
                    0
                };
            let salida = if ultimo && op.animacion != Animacion::Ninguna {
                120.min(dur_ms / 2)
            } else {
                0
            };
            if entrada > 0 || salida > 0 {
                tags.push_str(&format!("\\fad({entrada},{salida})"));
            }

            let cuerpo: Vec<String> = textos
                .iter()
                .enumerate()
                .map(|(k, t)| {
                    if Some(k) == activa && op.estilo != Estilo::Frase {
                        format!("{{\\c{color}}}{t}{{\\c&HFFFFFF&}}")
                    } else {
                        t.clone()
                    }
                })
                .collect();
            s.push_str(&format!(
                "Dialogue: 0,{},{},Caption,,0,0,0,,{{{tags}}}{}\n",
                marca_ass(a),
                marca_ass(b),
                cuerpo.join(" ")
            ));
        }
    }
    s
}

// ───────────────────────────── SRT ─────────────────────────────

/// Genera un .srt legible (frases de hasta ~42 caracteres) a partir de las palabras.
/// `desplazamiento` se suma a todos los tiempos (para alinearlo con el video original).
pub fn generar_srt(palabras: &[Palabra], mayusculas: bool, desplazamiento: f64) -> String {
    let frases = agrupar(palabras, 12, 42, 0.7);
    let mut s = String::new();
    for (i, frase) in frases.iter().enumerate() {
        let texto: Vec<String> = frase.iter().map(|p| p.texto.clone()).collect();
        let mut texto = texto.join(" ");
        if mayusculas {
            texto = texto.to_uppercase();
        }
        s.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            marca_srt(frase[0].ini + desplazamiento),
            marca_srt(fin_de_frase(&frases, i, 0.3) + desplazamiento),
            texto
        ));
    }
    s
}

/// Palabras de ejemplo para la vista previa del estilo.
pub fn palabras_de_ejemplo() -> Vec<Palabra> {
    ["Así", "se", "verán", "tus", "subtítulos"]
        .iter()
        .enumerate()
        .map(|(i, t)| Palabra {
            ini: i as f64 * 0.4,
            fin: i as f64 * 0.4 + 0.35,
            texto: t.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn p(ini: f64, fin: f64, t: &str) -> Palabra {
        Palabra {
            ini,
            fin,
            texto: t.into(),
        }
    }

    #[test]
    fn tramos_y_mapeo() {
        let log = "[silencedetect @ 0] silence_start: 0\n[silencedetect @ 0] silence_end: 3.1 | silence_duration: 3.1\n\
                   [silencedetect @ 0] silence_start: 10.0\n";
        let tramos = tramos_con_voz(log, 12.0);
        assert_eq!(tramos, vec![(2.95, 10.15)]);
        // 0,15 s del audio sin silencios = 3,10 s del clip
        assert!((tiempo_real(0.15, &tramos) - 3.10).abs() < 1e-9);
        assert_eq!(expresion_aselect(&tramos), "between(t\\,2.945\\,10.145)");
    }

    #[test]
    fn mapeo_con_varios_tramos() {
        let tramos = vec![(1.0, 2.0), (5.0, 7.0)];
        assert!((tiempo_real(0.5, &tramos) - 1.5).abs() < 1e-9);
        assert!((tiempo_real(1.5, &tramos) - 5.5).abs() < 1e-9);
    }

    #[test]
    fn json_filtra_alucinaciones() {
        let json = "{\"start\":100,\"end\":300,\"text\":\"Hola\"}\n{\"start\":300,\"end\":400,\"text\":\",\"}\n\
                    {\"start\":500,\"end\":900,\"text\":\"[Música]\"}\n{\"start\":900,\"end\":1200,\"text\":\"amigos\"}\n";
        let w = leer_palabras(json, &[]);
        assert_eq!(w, vec![p(0.1, 0.4, "Hola,"), p(0.9, 1.2, "amigos")]);
    }

    #[test]
    fn agrupa_por_pausas_y_puntuacion() {
        let w = vec![
            p(0.0, 0.3, "Hola"),
            p(0.3, 0.6, "a"),
            p(0.6, 0.9, "todos."),
            p(0.9, 1.2, "Hoy"),
            p(2.5, 2.8, "vamos"),
        ];
        let f = agrupar(&w, 3, 18, 0.45);
        let textos: Vec<Vec<&str>> = f
            .iter()
            .map(|f| f.iter().map(|p| p.texto.as_str()).collect())
            .collect();
        assert_eq!(
            textos,
            vec![vec!["Hola", "a", "todos."], vec!["Hoy"], vec!["vamos"]]
        );
    }

    #[test]
    fn ass_resalta_cada_palabra() {
        let op = Opciones {
            estilo: Estilo::Resaltado,
            fuente: Fuente::Poppins,
            animacion: Animacion::Pop,
            resaltado: ColorResaltado::Amarillo,
            tamano: Tamano::Mediano,
            posicion: Posicion::Abajo,
            mayusculas: true,
            quitar_puntuacion: true,
        };
        let ass = generar_ass(&[p(1.0, 1.4, "hola,"), p(1.5, 2.0, "mundo")], &op);
        let lineas: Vec<&str> = ass.lines().filter(|l| l.starts_with("Dialogue")).collect();
        assert_eq!(lineas.len(), 2);
        assert!(lineas[0].contains("0:00:01.00,0:00:01.50"));
        assert!(lineas[0].contains("{\\c&H00E6FF&}HOLA{\\c&HFFFFFF&} MUNDO"));
        assert!(lineas[0].contains("\\fscx70"));
        assert!(lineas[1].contains("HOLA {\\c&H00E6FF&}MUNDO"));
        assert!(lineas[1].contains("\\fad(0,120)"));
    }
}

#[cfg(test)]
mod pruebas_pausas {
    use super::*;

    #[test]
    fn palabra_no_cruza_pausa() {
        // Tramos de voz: 3,0–5,6 y 6,2–9,0. «canal» termina en el borde del audio
        // recortado (2,7 s), que caería dentro del segundo tramo.
        let tramos = vec![(3.0, 5.6), (6.2, 9.0)];
        let json = "{\"start\":2300,\"end\":2700,\"text\":\"canal.\"}\n";
        let w = leer_palabras(json, &tramos);
        assert!((w[0].ini - 5.3).abs() < 1e-9);
        assert!((w[0].fin - 5.6).abs() < 1e-9);
    }
}
