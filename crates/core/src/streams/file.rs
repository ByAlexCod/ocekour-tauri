//! Flux d'un fichier de logs (ou du plus récent d'un motif) : N dernières lignes lues depuis la
//! fin, « depuis » sur l'horodatage des lignes, suivi en direct (rotation, troncature,
//! suppression), encodages, `.gz`. Commun aux fichiers du poste et à la sonde : chaque lot
//! porte un curseur (fichier + position) qui permet de reprendre sans perte ni doublon.

use super::encoding::{ByteSplitter, Encoding, RawLine, detect_bom};
use super::pattern::{FileEntry, Pattern, io_error};
use super::{StreamEvent, StreamState};
use crate::error::{ErrorCode, SourceError};
use crate::logs::LogLine;
use base64::Engine;
use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

const BLOCK: usize = 64 * 1024;
/// « Depuis » sans limite de lignes : au plus, lignes remontées.
const MAX_HISTORY_LINES: u64 = 1_000_000;

/// Ce qu'on lit : un fichier, ou le plus récent des fichiers d'un motif.
#[derive(Debug, Clone)]
pub enum FileTarget {
    File(PathBuf),
    Pattern(Pattern),
}

impl FileTarget {
    /// Chemin saisi : motif s'il contient des jokers dans le nom, sinon fichier (absolu).
    pub fn parse(path: &str) -> Result<FileTarget, SourceError> {
        if super::pattern::is_pattern(path) { return Ok(FileTarget::Pattern(Pattern::parse(path)?)); }
        let p = PathBuf::from(path.trim());
        if !p.is_absolute() { return Err(SourceError::invalid("Chemin absolu attendu.")); }
        Ok(FileTarget::File(p))
    }

    pub fn display(&self) -> String {
        match self {
            FileTarget::File(p) => p.to_string_lossy().into_owned(),
            FileTarget::Pattern(p) => p.display(),
        }
    }
}

/// Position dans un fichier : identité (survit au renommage), nom, octet.
#[derive(Debug, Clone, PartialEq)]
pub struct Cursor {
    pub id: String,
    pub name: String,
    pub offset: u64,
}

impl Cursor {
    pub fn encode(&self) -> String {
        format!("{}:{}:{}", self.offset, self.id, base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&self.name))
    }

    pub fn decode(s: &str) -> Option<Cursor> {
        let mut parts = s.splitn(3, ':');
        let offset = parts.next()?.parse().ok()?;
        let id = parts.next()?.to_string();
        let name = String::from_utf8(base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts.next()?).ok()?).ok()?;
        if id.is_empty() { return None; }
        Some(Cursor { id, name, offset })
    }
}

#[derive(Debug, Clone, Default)]
pub struct FileOptions {
    pub follow: bool,
    pub tail_lines: Option<u64>,
    pub since_seconds: Option<u64>,
    /// Encodage imposé ; sinon détecté (BOM), UTF-8 par défaut.
    pub encoding: Option<Encoding>,
    /// Reprise après coupure (sonde) : pas d'historique, on repart du curseur.
    pub resume: Option<Cursor>,
}

#[derive(Debug, Clone)]
pub struct FileTiming {
    /// Intervalle de vérification du fichier (ajouts, rotation).
    pub poll: Duration,
    /// Ligne sans fin de ligne envoyée après ce délai sans écriture.
    pub idle_flush: Duration,
    pub batch_lines: usize,
}

impl Default for FileTiming {
    fn default() -> Self { FileTiming { poll: Duration::from_millis(250), idle_flush: Duration::from_secs(1), batch_lines: 1_000 } }
}

/// Événement du flux, avec la position atteinte (lots de lignes).
#[derive(Debug, Clone)]
pub struct FileEvent {
    pub event: StreamEvent,
    pub cursor: Option<Cursor>,
}

// --- Accès aux fichiers ----------------------------------------------------------------------

/// Ouverture en lecture partagée : ne gêne ni l'écriture, ni la rotation, ni la suppression.
pub fn open_shared(path: &Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
        options.share_mode(0x1 | 0x2 | 0x4);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() { return Err(std::io::Error::other("ce n'est pas un fichier")); }
    Ok(file)
}

/// Identité d'un fichier ouvert (inode / index NTFS) : distingue un fichier renouvelé.
pub fn identity(file: &File) -> std::io::Result<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        Ok(format!("{:x}-{:x}", m.dev(), m.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(format!("{:x}-{:x}{:08x}", info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow))
    }
}

fn path_identity(path: &Path) -> std::io::Result<(String, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path)?;
        if !m.is_file() { return Err(std::io::Error::other("ce n'est pas un fichier")); }
        Ok((format!("{:x}-{:x}", m.dev(), m.ino()), m.len()))
    }
    #[cfg(not(unix))]
    {
        let f = open_shared(path)?;
        Ok((identity(&f)?, f.metadata()?.len()))
    }
}

fn is_gz(path: &Path) -> bool { path.extension().is_some_and(|e| e.eq_ignore_ascii_case("gz")) }

fn name_of(path: &Path) -> String { path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() }

/// Encodage effectif et début des données (après la marque d'ordre des octets).
fn detect(file: &mut File, forced: Option<Encoding>) -> std::io::Result<(Encoding, u64)> {
    let mut head = [0u8; 3];
    file.seek(SeekFrom::Start(0))?;
    let n = file.read(&mut head)?;
    let bom = detect_bom(&head[..n]);
    Ok(match (forced, bom) {
        (Some(e), Some((b, len))) if e.unit() == b.unit() => (e, len),
        (Some(e), _) => (e, 0),
        (None, Some((b, len))) => (b, len),
        (None, None) => (Encoding::Utf8, 0),
    })
}

// --- Horodatage des lignes -------------------------------------------------------------------

static LEADING_TS: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(r"^\[?(\d{4}-\d\d-\d\d[T ]\d\d:\d\d:\d\d(?:[.,]\d{1,9})?(?:\s?(?:Z|[+-]\d\d:?\d\d))?)").unwrap()
});

/// Horodatage d'une ligne : champ de date d'une ligne JSON, ou date en début de ligne.
pub fn line_time(text: &str) -> Option<DateTime<Utc>> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') {
        let parsed = crate::logs::parser::parse_line(trimmed, None, &Default::default());
        if parsed.time.is_some() { return parsed.time; }
    }
    let m = LEADING_TS.captures(trimmed)?;
    crate::logs::parser::to_date_str(&m[1])
}

fn to_log_line(text: String) -> LogLine {
    let ts = line_time(&text).map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    LogLine { ts, text, meta: None }
}

// --- Historique : lecture depuis la fin --------------------------------------------------------

/// Parcourt les lignes de `[data_start, end)` de la dernière à la première ; `visit(début,
/// octets)` renvoie `false` pour arrêter.
fn scan_backward(file: &mut File, data_start: u64, end: u64, encoding: Encoding, mut visit: impl FnMut(u64, &[u8]) -> bool) -> std::io::Result<()> {
    let unit = encoding.unit() as u64;
    if end <= data_start { return Ok(()); }
    // Fin de la dernière ligne (sans sa fin de ligne éventuelle).
    let mut line_end = end;
    let mut last = vec![0u8; unit as usize];
    if end - data_start >= unit {
        file.seek(SeekFrom::Start(end - unit))?;
        file.read_exact(&mut last)?;
        if encoding.is_newline(&last, 0) { line_end = end - unit; }
    }
    // `buf` = octets de `[buf_start, line_end)` pas encore rattachés à une ligne.
    let mut buf: Vec<u8> = vec![];
    let mut buf_start = line_end;
    let mut pos = line_end;
    loop {
        if pos > data_start {
            let mut from = pos.saturating_sub(BLOCK as u64).max(data_start);
            from += (from - data_start) % unit; // alignement UTF-16
            if from >= pos { from = data_start; }
            let mut block = vec![0u8; (pos - from) as usize];
            file.seek(SeekFrom::Start(from))?;
            file.read_exact(&mut block)?;
            block.extend_from_slice(&buf);
            buf = block;
            buf_start = from;
            pos = from;
        }
        // Fins de ligne dans `buf`, de la dernière à la première.
        // Positions alignées sur l'unité (`buf_start` l'est).
        let mut i = (buf.len() as i64 - unit as i64).div_euclid(unit as i64) * unit as i64;
        let mut cut = buf.len();
        while i >= 0 {
            let iu = i as usize;
            if encoding.is_newline(&buf, iu) {
                let start = buf_start + iu as u64 + unit;
                if !visit(start, &buf[iu + unit as usize..cut]) { return Ok(()); }
                cut = iu;
            }
            i -= unit as i64;
        }
        buf.truncate(cut);
        if pos <= data_start {
            if line_end > data_start || !buf.is_empty() { visit(data_start, &buf); }
            return Ok(());
        }
    }
}

/// Début des lignes à relire : N dernières, et/ou postérieures à `since`. Renvoie aussi le
/// nombre de lignes retenues et si la limite a été atteinte (rien à chercher avant).
fn history_start(file: &mut File, data_start: u64, end: u64, encoding: Encoding, max_lines: Option<u64>, since: Option<DateTime<Utc>>) -> std::io::Result<(u64, u64, bool)> {
    let max = max_lines.unwrap_or(MAX_HISTORY_LINES);
    let mut start = end;
    let mut dated_start = end;
    let mut count = 0u64;
    let mut done = false;
    scan_backward(file, data_start, end, encoding, |line_start, bytes| {
        if let Some(limit) = since {
            if let Some(t) = line_time(&encoding.decode(bytes)) {
                if t < limit {
                    // Les lignes non datées qui suivent appartiennent à cette ligne ancienne.
                    start = dated_start;
                    done = true;
                    return false;
                }
                dated_start = line_start;
            }
        }
        count += 1;
        start = line_start;
        if count >= max { done = true; return false; }
        true
    })?;
    Ok((start, count, done))
}

// --- Le flux -------------------------------------------------------------------------------

/// Fichier en cours de lecture.
struct Current {
    path: PathBuf,
    name: String,
    file: File,
    id: String,
    splitter: ByteSplitter,
    /// Octets lus (complets ou en attente dans le découpeur).
    read: u64,
    last_growth: Instant,
}

impl Current {
    fn open(path: &Path, forced: Option<Encoding>, at: Option<u64>) -> std::io::Result<Current> {
        let mut file = open_shared(path)?;
        let (encoding, data_start) = detect(&mut file, forced)?;
        let id = identity(&file)?;
        let start = at.unwrap_or(data_start).max(data_start);
        Ok(Current {
            path: path.to_path_buf(),
            name: name_of(path),
            file,
            id,
            splitter: ByteSplitter::new(encoding, start),
            read: start,
            last_growth: Instant::now(),
        })
    }

    fn len(&self) -> u64 { self.file.metadata().map(|m| m.len()).unwrap_or(0) }

    /// Lit jusqu'à `until` (ou la fin) ; renvoie les lignes complètes.
    fn read_more(&mut self, until: Option<u64>) -> std::io::Result<Vec<RawLine>> {
        let end = until.unwrap_or_else(|| self.len());
        let mut lines = vec![];
        if end <= self.read { return Ok(lines); }
        self.file.seek(SeekFrom::Start(self.read))?;
        let mut left = end - self.read;
        let mut buf = vec![0u8; (256 * 1024).min(left as usize)];
        while left > 0 {
            let want = (buf.len() as u64).min(left) as usize;
            let n = self.file.read(&mut buf[..want])?;
            if n == 0 { break; }
            self.read += n as u64;
            left -= n as u64;
            lines.extend(self.splitter.push(&buf[..n]));
        }
        self.last_growth = Instant::now();
        Ok(lines)
    }
}

struct Run {
    target: FileTarget,
    options: FileOptions,
    timing: FileTiming,
    cancel: CancellationToken,
    tx: mpsc::Sender<FileEvent>,
}

/// Lance la lecture dans un fil dédié ; les événements arrivent dans `tx` (la lecture attend
/// quand le destinataire est lent). S'arrête à la fin (historique), quand `cancel` est
/// déclenché ou quand le destinataire est fermé.
pub fn spawn(target: FileTarget, options: FileOptions, timing: FileTiming, cancel: CancellationToken, tx: mpsc::Sender<FileEvent>) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || Run { target, options, timing, cancel, tx }.main())
}

impl Run {
    fn stopped(&self) -> bool { self.cancel.is_cancelled() || self.tx.is_closed() }

    fn send(&self, event: StreamEvent, cursor: Option<Cursor>) -> bool {
        !self.cancel.is_cancelled() && self.tx.blocking_send(FileEvent { event, cursor }).is_ok()
    }

    fn status(&self, state: StreamState, message: Option<String>) -> bool { self.send(StreamEvent::status(state, message), None) }

    fn notice(&self, message: impl Into<String>) -> bool { self.send(StreamEvent::Notice { message: message.into() }, None) }

    fn fail(&self, error: SourceError) {
        let _ = self.send(StreamEvent::Status { state: StreamState::Error, message: Some(error.message.clone()), error: Some(error), replacements: None }, None);
    }

    /// Envoie des lignes par lots ; `false` si le flux doit s'arrêter.
    fn lines(&self, lines: Vec<RawLine>, cursor: impl Fn(u64) -> Cursor) -> bool {
        for chunk in lines.chunks(self.timing.batch_lines.max(1)) {
            let end = chunk.last().map(|l| l.end).unwrap_or(0);
            let batch = chunk.iter().map(|l| to_log_line(l.text.clone())).collect();
            if !self.send(StreamEvent::Lines { lines: batch }, Some(cursor(end))) { return false; }
        }
        true
    }

    /// Fichier le plus récent visé (fichier seul : lui-même s'il existe).
    fn latest(&self) -> Result<Option<FileEntry>, SourceError> {
        match &self.target {
            FileTarget::File(p) => match FileEntry::of(p) {
                Ok(e) => Ok(Some(e)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(io_error(p, &e)),
            },
            FileTarget::Pattern(p) => Ok(p.list(1)?.into_iter().next()),
        }
    }

    fn files(&self) -> Result<Vec<FileEntry>, SourceError> {
        match &self.target {
            FileTarget::File(_) => Ok(self.latest()?.into_iter().collect()),
            FileTarget::Pattern(p) => p.list(super::pattern::MAX_FILES),
        }
    }

    fn main(self) {
        if !self.status(StreamState::Connecting, None) { return; }
        let current = match self.options.resume.clone() {
            Some(cursor) => self.resume(cursor),
            None => self.history(),
        };
        let current = match current {
            Ok(Some(c)) => c,
            Ok(None) => return,
            Err(e) => return self.fail(e),
        };
        if !self.options.follow {
            let _ = self.status(StreamState::Ended, None);
            return;
        }
        self.follow(current);
    }

    /// Attend qu'un fichier existe (suivi) ; `None` si arrêté ou si l'historique seul est demandé.
    fn wait_for_file(&self) -> Result<Option<FileEntry>, SourceError> {
        let mut announced = false;
        loop {
            if let Some(f) = self.latest()? { return Ok(Some(f)); }
            if !self.options.follow {
                let what = match &self.target {
                    FileTarget::File(p) => format!("{} : introuvable.", p.display()),
                    FileTarget::Pattern(p) => format!("Aucun fichier ne correspond à {}.", p.display()),
                };
                return Err(SourceError::new(ErrorCode::NotFound, what));
            }
            if !announced {
                announced = true;
                let message = match &self.target {
                    FileTarget::File(_) => "Fichier absent : en attente de sa création",
                    FileTarget::Pattern(_) => "Aucun fichier ne correspond au motif : en attente",
                };
                if !self.status(StreamState::Reconnecting, Some(message.into())) { return Ok(None); }
            }
            std::thread::sleep(self.timing.poll);
            if self.stopped() { return Ok(None); }
        }
    }

    fn history(&self) -> Result<Option<Current>, SourceError> {
        let Some(newest) = self.wait_for_file()? else { return Ok(None) };
        let since = self.options.since_seconds.map(|s| Utc::now() - chrono::Duration::seconds(s as i64));
        let all = self.options.tail_lines.is_none() && since.is_none();

        if is_gz(&newest.path) { self.read_gz(&newest.path, since)?; return Ok(None); }

        // Segments à relire (du plus ancien au plus récent) : un motif remonte dans les
        // fichiers précédents tant qu'il manque des lignes.
        let mut segments: Vec<(PathBuf, u64)> = vec![];
        let mut current: Option<Current> = None;
        let mut wanted = self.options.tail_lines;
        let candidates = if all { vec![newest.clone()] } else { self.files()? };
        for (i, entry) in candidates.iter().enumerate() {
            if i > 0 && is_gz(&entry.path) { break; }
            let mut file = open_shared(&entry.path).map_err(|e| io_error(&entry.path, &e))?;
            let (encoding, data_start) = detect(&mut file, self.options.encoding).map_err(|e| io_error(&entry.path, &e))?;
            let end = file.metadata().map(|m| m.len()).unwrap_or(0);
            let (start, count, done) = if all {
                (data_start, 0, true)
            } else {
                history_start(&mut file, data_start, end, encoding, wanted, since).map_err(|e| io_error(&entry.path, &e))?
            };
            if i == 0 {
                current = Some(Current::open(&entry.path, self.options.encoding, Some(start)).map_err(|e| io_error(&entry.path, &e))?);
            } else {
                segments.push((entry.path.clone(), start));
            }
            if let Some(w) = wanted.as_mut() { *w = w.saturating_sub(count); }
            if done || wanted == Some(0) { break; }
        }
        // Fichiers précédents (complets), du plus ancien au plus récent.
        for (path, start) in segments.into_iter().rev() {
            let mut c = Current::open(&path, self.options.encoding, Some(start)).map_err(|e| io_error(&path, &e))?;
            let mut lines = c.read_more(None).map_err(|e| io_error(&path, &e))?;
            lines.extend(c.splitter.flush());
            let (id, name) = (c.id.clone(), c.name.clone());
            if !self.lines(lines, |offset| Cursor { id: id.clone(), name: name.clone(), offset }) { return Ok(None); }
        }
        let mut c = current.expect("fichier courant");
        let end = c.len();
        let mut lines = c.read_more(Some(end)).map_err(|e| io_error(&c.path, &e))?;
        if !self.options.follow { lines.extend(c.splitter.flush()); }
        let (id, name) = (c.id.clone(), c.name.clone());
        if !self.lines(lines, |offset| Cursor { id: id.clone(), name: name.clone(), offset }) { return Ok(None); }
        Ok(Some(c))
    }

    /// `.gz` : décompression en flux, N dernières lignes (pas de suivi).
    fn read_gz(&self, path: &Path, since: Option<DateTime<Utc>>) -> Result<(), SourceError> {
        let file = open_shared(path).map_err(|e| io_error(path, &e))?;
        let mut gz = flate2::read::MultiGzDecoder::new(std::io::BufReader::new(file));
        let mut head = vec![0u8; 3];
        let n = gz.read(&mut head).map_err(|e| io_error(path, &e))?;
        head.truncate(n);
        let (encoding, skip) = match (self.options.encoding, detect_bom(&head)) {
            (Some(e), _) => (e, detect_bom(&head).map(|b| b.1).unwrap_or(0)),
            (None, Some((e, len))) => (e, len),
            (None, None) => (Encoding::Utf8, 0),
        };
        let mut splitter = ByteSplitter::new(encoding, 0);
        let max = self.options.tail_lines.unwrap_or(u64::MAX) as usize;
        let mut kept: VecDeque<RawLine> = VecDeque::new();
        let keep = |line: RawLine, kept: &mut VecDeque<RawLine>| {
            if let Some(limit) = since {
                if let Some(t) = line_time(&line.text) {
                    if t < limit { kept.clear(); return; }
                } else if kept.is_empty() {
                    return;
                }
            }
            kept.push_back(line);
            if kept.len() > max { kept.pop_front(); }
        };
        for line in splitter.push(&head[skip as usize..]) { keep(line, &mut kept); }
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if self.stopped() { return Ok(()); }
            let n = gz.read(&mut buf).map_err(|e| SourceError::invalid(format!("{} : archive illisible ({e}).", path.display())))?;
            if n == 0 { break; }
            for line in splitter.push(&buf[..n]) { keep(line, &mut kept); }
        }
        if let Some(line) = splitter.flush() { keep(line, &mut kept); }
        let name = name_of(path);
        self.lines(kept.into(), |offset| Cursor { id: "gz".into(), name: name.clone(), offset });
        if self.options.follow { self.notice("Fichier compressé : pas de suivi en direct."); }
        Ok(())
    }

    /// Reprise après coupure : on repart du curseur, sans historique.
    fn resume(&self, cursor: Cursor) -> Result<Option<Current>, SourceError> {
        let Some(newest) = self.wait_for_file()? else { return Ok(None) };
        // Fichier du curseur, retrouvé par son identité (il a pu être renommé).
        let files = self.files()?;
        let found = files.iter().position(|f| path_identity(&f.path).map(|(id, _)| id == cursor.id).unwrap_or(false));
        let Some(index) = found else {
            if !self.notice("Fichier renouvelé pendant la coupure : relecture du nouveau fichier") { return Ok(None); }
            let mut c = Current::open(&newest.path, self.options.encoding, None).map_err(|e| io_error(&newest.path, &e))?;
            let lines = c.read_more(None).map_err(|e| io_error(&c.path, &e))?;
            let (id, name) = (c.id.clone(), c.name.clone());
            if !self.lines(lines, |offset| Cursor { id: id.clone(), name: name.clone(), offset }) { return Ok(None); }
            return Ok(Some(c));
        };
        // Reste du fichier du curseur, puis les fichiers plus récents apparus entre temps.
        let mut chain: Vec<&FileEntry> = files[..=index].iter().collect();
        chain.reverse();
        let last = chain.len() - 1;
        let mut current = None;
        for (i, entry) in chain.into_iter().enumerate() {
            let at = if i == 0 { Some(cursor.offset) } else { None };
            let mut c = Current::open(&entry.path, self.options.encoding, at).map_err(|e| io_error(&entry.path, &e))?;
            if i == 0 && c.len() < cursor.offset {
                if !self.notice("Fichier tronqué pendant la coupure : relecture depuis le début") { return Ok(None); }
                c = Current::open(&entry.path, self.options.encoding, None).map_err(|e| io_error(&entry.path, &e))?;
            }
            if i > 0 && !self.notice(format!("Nouveau fichier : {}", c.name)) { return Ok(None); }
            let mut lines = c.read_more(None).map_err(|e| io_error(&c.path, &e))?;
            if i < last { lines.extend(c.splitter.flush()); }
            let (id, name) = (c.id.clone(), c.name.clone());
            if !self.lines(lines, |offset| Cursor { id: id.clone(), name: name.clone(), offset }) { return Ok(None); }
            current = Some(c);
        }
        Ok(current)
    }

    fn emit_current(&self, c: &mut Current, flush: bool) -> bool {
        let mut lines = match c.read_more(None) {
            Ok(l) => l,
            Err(e) => {
                log::debug!("Lecture de {} : {e}", c.path.display());
                vec![]
            }
        };
        if flush { lines.extend(c.splitter.flush()); }
        let (id, name) = (c.id.clone(), c.name.clone());
        self.lines(lines, |offset| Cursor { id: id.clone(), name: name.clone(), offset })
    }

    fn follow(&self, mut c: Current) {
        if !self.status(StreamState::Live, None) { return; }
        let mut missing = false;
        loop {
            std::thread::sleep(self.timing.poll);
            if self.stopped() { return; }
            let before = c.read;
            if !self.emit_current(&mut c, false) { return; }
            // Dernière ligne sans fin de ligne : envoyée après un moment sans écriture.
            if c.read == before && c.splitter.pending() > 0 && c.last_growth.elapsed() >= self.timing.idle_flush {
                if let Some(line) = c.splitter.flush() {
                    let (id, name) = (c.id.clone(), c.name.clone());
                    if !self.lines(vec![line], |offset| Cursor { id: id.clone(), name: name.clone(), offset }) { return; }
                }
            }
            let latest = match self.latest() {
                Ok(l) => l,
                Err(e) => {
                    log::debug!("Suivi de {} : {}", self.target.display(), e.message);
                    continue;
                }
            };
            let Some(latest) = latest else {
                if !missing {
                    missing = true;
                    let message = match &self.target {
                        FileTarget::File(_) => "Fichier absent : en attente de sa création",
                        FileTarget::Pattern(_) => "Plus aucun fichier ne correspond au motif : en attente",
                    };
                    if !self.status(StreamState::Reconnecting, Some(message.into())) { return; }
                }
                continue;
            };
            let Ok((id, len)) = path_identity(&latest.path) else { continue };
            if id != c.id {
                // Rotation, ou fichier plus récent dans le motif : on termine l'ancien, puis
                // on lit le nouveau depuis le début.
                if !self.emit_current(&mut c, true) { return; }
                let next = match Current::open(&latest.path, self.options.encoding, None) {
                    Ok(n) => n,
                    Err(_) => continue,
                };
                let message = match (&self.target, missing) {
                    (_, true) => "Fichier recréé".to_string(),
                    (FileTarget::File(_), false) => "Fichier renouvelé (rotation)".to_string(),
                    (FileTarget::Pattern(_), false) => format!("Nouveau fichier : {}", next.name),
                };
                c = next;
                if !self.notice(message) { return; }
                if missing {
                    missing = false;
                    if !self.status(StreamState::Live, None) { return; }
                }
                if !self.emit_current(&mut c, false) { return; }
            } else {
                if missing {
                    missing = false;
                    if !self.status(StreamState::Live, None) { return; }
                }
                if len < c.read {
                    let _ = c.splitter.flush();
                    match Current::open(&c.path, self.options.encoding, None) {
                        Ok(n) => c = n,
                        Err(_) => continue,
                    }
                    if !self.notice("Fichier tronqué : relecture depuis le début") { return; }
                    if !self.emit_current(&mut c, false) { return; }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn timing() -> FileTiming { FileTiming { poll: Duration::from_millis(20), idle_flush: Duration::from_millis(100), batch_lines: 1_000 } }

    async fn collect(target: FileTarget, options: FileOptions) -> (Vec<String>, Vec<String>, Option<Cursor>, Vec<StreamState>) {
        let (tx, mut rx) = mpsc::channel(16);
        spawn(target, options, timing(), CancellationToken::new(), tx);
        let (mut lines, mut notices, mut cursor, mut states) = (vec![], vec![], None, vec![]);
        while let Some(e) = rx.recv().await {
            match e.event {
                StreamEvent::Lines { lines: l } => { lines.extend(l.into_iter().map(|l| l.text)); cursor = e.cursor; }
                StreamEvent::Notice { message } => notices.push(message),
                StreamEvent::Status { state, .. } => states.push(state),
            }
        }
        (lines, notices, cursor, states)
    }

    fn write(path: &Path, text: &[u8]) { std::fs::write(path, text).unwrap(); }
    fn append(path: &Path, text: &str) { std::fs::OpenOptions::new().append(true).open(path).unwrap().write_all(text.as_bytes()).unwrap(); }

    #[tokio::test]
    async fn dernieres_lignes_et_tout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        // > 10 Mo : la lecture depuis la fin ne lit que les derniers blocs.
        let mut text = String::new();
        for i in 0..200_000 { text.push_str(&format!("2026-10-22 10:00:00,000 INFO ligne numéro {i:06} avec un peu de texte\r\n")); }
        write(&path, text.as_bytes());
        assert!(text.len() > 10_000_000);
        let started = Instant::now();
        let (lines, _, cursor, states) = collect(FileTarget::File(path.clone()), FileOptions { tail_lines: Some(3), ..Default::default() }).await;
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(lines.len(), 3);
        assert!(lines[2].ends_with("ligne numéro 199999 avec un peu de texte"), "{lines:?}");
        assert_eq!(states, [StreamState::Connecting, StreamState::Ended]);
        assert_eq!(cursor.unwrap().offset, text.len() as u64);

        // Sans fin de ligne finale, la dernière ligne est lue aussi.
        let small = dir.path().join("petit.log");
        write(&small, b"a\nb\nc");
        let (lines, ..) = collect(FileTarget::File(small.clone()), FileOptions { tail_lines: Some(2), ..Default::default() }).await;
        assert_eq!(lines, ["b", "c"]);
        let (lines, ..) = collect(FileTarget::File(small.clone()), FileOptions::default()).await;
        assert_eq!(lines, ["a", "b", "c"]);
        let (lines, ..) = collect(FileTarget::File(small), FileOptions { tail_lines: Some(10), ..Default::default() }).await;
        assert_eq!(lines, ["a", "b", "c"]);
        let empty = dir.path().join("vide.log");
        write(&empty, b"");
        let (lines, _, _, states) = collect(FileTarget::File(empty), FileOptions { tail_lines: Some(10), ..Default::default() }).await;
        assert!(lines.is_empty() && states.last() == Some(&StreamState::Ended));
    }

    #[tokio::test]
    async fn depuis_sur_l_horodatage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        let old = (Utc::now() - chrono::Duration::hours(2)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let recent = (Utc::now() - chrono::Duration::seconds(30)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        write(&path, format!("{old} ancienne\n  at trace ancienne\n{recent} récente\n  at trace récente\n{{\"@timestamp\":\"{recent}\",\"message\":\"json\"}}\n").as_bytes());
        let (lines, ..) = collect(FileTarget::File(path.clone()), FileOptions { since_seconds: Some(600), ..Default::default() }).await;
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].ends_with("récente") && lines[1] == "  at trace récente");
        let (lines, ..) = collect(FileTarget::File(path), FileOptions { since_seconds: Some(600), tail_lines: Some(1), ..Default::default() }).await;
        assert_eq!(lines.len(), 1);
        // Horodatages reconnus.
        assert!(line_time("2026-10-22 10:00:00,123 INFO x").is_some());
        assert!(line_time("[2026-10-22T10:00:00.123Z] x").is_some());
        assert!(line_time(r#"{"time":"2026-10-22T10:00:00Z"}"#).is_some());
        assert!(line_time("sans date").is_none());
    }

    #[tokio::test]
    async fn encodages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("windows.log");
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("première\r\nété ☃\r\n".encode_utf16().flat_map(|u| u.to_le_bytes()));
        write(&path, &bytes);
        let (lines, ..) = collect(FileTarget::File(path), FileOptions { tail_lines: Some(1), ..Default::default() }).await;
        assert_eq!(lines, ["été ☃"]);
        let (lines, ..) = collect(FileTarget::File(dir.path().join("windows.log")), FileOptions::default()).await;
        assert_eq!(lines, ["première", "été ☃"]);

        let path = dir.path().join("cp1252.log");
        write(&path, &[0x80, b' ', 0x9C, b' ', 0x93, b'x', 0x94, b'\n', 0xE9, 0xFF, b'\n']);
        let (lines, ..) = collect(FileTarget::File(path.clone()), FileOptions { encoding: Some(Encoding::Windows1252), ..Default::default() }).await;
        assert_eq!(lines, ["€ œ “x”", "éÿ"]);
        let (lines, ..) = collect(FileTarget::File(path.clone()), FileOptions { encoding: Some(Encoding::Latin1), ..Default::default() }).await;
        assert_eq!(lines[1], "éÿ");
        let (lines, ..) = collect(FileTarget::File(path), FileOptions::default()).await;
        assert_eq!(lines[1], "\u{FFFD}\u{FFFD}");
    }

    #[tokio::test]
    async fn gz() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log.gz");
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        for i in 0..1000 { writeln!(gz, "ligne {i}").unwrap(); }
        write(&path, &gz.finish().unwrap());
        let (lines, notices, ..) = collect(FileTarget::File(path), FileOptions { tail_lines: Some(2), follow: true, ..Default::default() }).await;
        assert_eq!(lines, ["ligne 998", "ligne 999"]);
        assert_eq!(notices, ["Fichier compressé : pas de suivi en direct."]);
    }

    async fn next_lines(rx: &mut mpsc::Receiver<FileEvent>, notices: &mut Vec<String>) -> (Vec<String>, Option<Cursor>) {
        loop {
            let e = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("délai").expect("flux fermé");
            match e.event {
                StreamEvent::Lines { lines } => return (lines.into_iter().map(|l| l.text).collect(), e.cursor),
                StreamEvent::Notice { message } => notices.push(message),
                StreamEvent::Status { .. } => {}
            }
        }
    }

    #[tokio::test]
    async fn suivi_rotation_troncature_suppression() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.log");
        write(&path, b"un\n");
        let (tx, mut rx) = mpsc::channel(16);
        let cancel = CancellationToken::new();
        spawn(FileTarget::File(path.clone()), FileOptions { follow: true, tail_lines: Some(10), ..Default::default() }, timing(), cancel.clone(), tx);
        let mut notices = vec![];
        assert_eq!(next_lines(&mut rx, &mut notices).await.0, ["un"]);
        append(&path, "deux\ntrois");
        assert_eq!(next_lines(&mut rx, &mut notices).await.0, ["deux"]);
        // Ligne sans fin : envoyée après un moment sans écriture.
        assert_eq!(next_lines(&mut rx, &mut notices).await.0, ["trois"]);
        append(&path, "\nquatre\n");
        assert_eq!(next_lines(&mut rx, &mut notices).await.0, ["quatre"]);

        // Rotation : renommage puis nouveau fichier ; la fin de l'ancien est lue d'abord.
        append(&path, "fin de l'ancien\n");
        std::fs::rename(&path, dir.path().join("app.log.1")).unwrap();
        write(&path, b"nouveau\n");
        let mut got = vec![];
        while !got.contains(&"nouveau".to_string()) { got.extend(next_lines(&mut rx, &mut notices).await.0); }
        assert_eq!(got, ["fin de l'ancien", "nouveau"]);
        assert!(notices.contains(&"Fichier renouvelé (rotation)".to_string()), "{notices:?}");

        // Troncature.
        std::fs::OpenOptions::new().write(true).truncate(true).open(&path).unwrap();
        append(&path, "après\n");
        assert_eq!(next_lines(&mut rx, &mut notices).await.0, ["après"]);
        assert!(notices.iter().any(|n| n.contains("tronqué")), "{notices:?}");

        // Suppression puis recréation.
        std::fs::remove_file(&path).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        write(&path, "recréé\n".as_bytes());
        assert_eq!(next_lines(&mut rx, &mut notices).await.0, ["recréé"]);
        assert!(notices.contains(&"Fichier recréé".to_string()), "{notices:?}");
        cancel.cancel();
    }

    #[tokio::test]
    async fn fichier_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.log");
        let (lines, _, _, states) = collect(FileTarget::File(path.clone()), FileOptions::default()).await;
        assert!(lines.is_empty());
        assert_eq!(states, [StreamState::Connecting, StreamState::Error]);
        // En suivi : attente puis lecture dès la création.
        let (tx, mut rx) = mpsc::channel(16);
        spawn(FileTarget::File(path.clone()), FileOptions { follow: true, tail_lines: Some(10), ..Default::default() }, timing(), CancellationToken::new(), tx);
        tokio::time::sleep(Duration::from_millis(100)).await;
        write(&path, b"arrivee\n");
        let mut notices = vec![];
        assert_eq!(next_lines(&mut rx, &mut notices).await.0, ["arrivee"]);
    }

    #[tokio::test]
    async fn motif_historique_bascule_et_reprise() {
        let dir = tempfile::tempdir().unwrap();
        let set = |name: &str, text: &str, secs: u64| {
            let p = dir.path().join(name);
            write(&p, text.as_bytes());
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(std::time::SystemTime::now() - Duration::from_secs(secs)).unwrap();
            p
        };
        set("batch-1.log", "a1\na2\n", 300);
        set("batch-2.log", "b1\nb2\n", 200);
        set("autre.log", "zz\n", 100);
        let pattern = Pattern::parse(&format!("{}/batch-*.log", dir.path().display())).unwrap();
        // N lignes à cheval sur deux fichiers.
        let (lines, ..) = collect(FileTarget::Pattern(pattern.clone()), FileOptions { tail_lines: Some(3), ..Default::default() }).await;
        assert_eq!(lines, ["a2", "b1", "b2"]);
        // « Tout » : le fichier le plus récent seul.
        let (lines, ..) = collect(FileTarget::Pattern(pattern.clone()), FileOptions::default()).await;
        assert_eq!(lines, ["b1", "b2"]);

        // Suivi : bascule sur un nouveau fichier.
        let (tx, mut rx) = mpsc::channel(16);
        let cancel = CancellationToken::new();
        spawn(FileTarget::Pattern(pattern.clone()), FileOptions { follow: true, tail_lines: Some(1), ..Default::default() }, timing(), cancel.clone(), tx);
        let mut notices = vec![];
        let (first, cursor) = next_lines(&mut rx, &mut notices).await;
        assert_eq!(first, ["b2"]);
        let cursor = cursor.unwrap();
        assert_eq!(cursor.name, "batch-2.log");
        append(&dir.path().join("batch-2.log"), "b3\n");
        tokio::time::sleep(Duration::from_millis(100)).await;
        write(&dir.path().join("batch-3.log"), b"c1\n");
        let mut got = vec![];
        while !got.contains(&"c1".to_string()) { got.extend(next_lines(&mut rx, &mut notices).await.0); }
        assert_eq!(got, ["b3", "c1"]);
        assert!(notices.contains(&"Nouveau fichier : batch-3.log".to_string()), "{notices:?}");
        cancel.cancel();

        // Reprise depuis le curseur pris sur batch-2 : ni perte ni doublon.
        let (lines, notices, ..) = collect(FileTarget::Pattern(pattern), FileOptions { resume: Some(cursor), ..Default::default() }).await;
        assert_eq!(lines, ["b3", "c1"]);
        assert_eq!(notices, ["Nouveau fichier : batch-3.log"]);
    }

    #[test]
    fn curseur() {
        let c = Cursor { id: "803-1a2b".into(), name: "app:é.log".into(), offset: 42 };
        assert_eq!(Cursor::decode(&c.encode()), Some(c));
        assert_eq!(Cursor::decode("x:y"), None);
        assert_eq!(Cursor::decode("1::"), None);
    }
}
