//! One version of an item: its files, checked against its kind before anything keeps them,
//! and packed in one canonical form whose sha256 is the version's digest.
//!
//! The canonical form is a POSIX ustar archive holding nothing that varies between machines:
//! regular files only, sorted by path, every time and owner 0, and mode 0644 or 0755 as the
//! executable bit says. So the same files make the same bytes, and the same digest, wherever
//! they came from — a console upload, a forge's archive, a directory on a laptop — and
//! `tar tvf` reads a blob as it is.

use std::collections::HashSet;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::{check_name, Kind, Refused};

/// The most one file may hold.
pub const FILE_MAX: usize = 1024 * 1024;
/// The most one version may hold, all its files together. A skill is instructions and a few
/// scripts; anything bigger is a mistake — a virtual environment, a build — that would be
/// copied to every machine.
pub const BUNDLE_MAX: usize = 8 * 1024 * 1024;

/// A machine's own clutter, never part of a component: packing a directory leaves these out
/// wherever they appear. A skill on the owner's Mac carried a whole Python virtual
/// environment in `.venv`.
const LEFT_OUT: [&str; 6] = [".git", ".DS_Store", "__pycache__", ".venv", "venv", "node_modules"];

/// One file of a bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// Relative to the item, `/`-separated: `SKILL.md`, `scripts/run.sh`, a command's `<name>.md`.
    pub path: String,
    pub bytes: Vec<u8>,
    /// The only part of a file's mode a bundle keeps.
    pub executable: bool,
}

/// A bundle that passed its kind's checks: its files sorted by path, and the description its
/// frontmatter gives, which is what Claude Code lists the item by.
#[derive(Clone, Debug)]
pub struct Bundle {
    kind: Kind,
    name: String,
    files: Vec<File>,
    description: String,
}

impl Bundle {
    /// Check `files` as one version of the item `kind`/`name`, or say why they cannot be one.
    ///
    /// - Every path is relative and stays inside the item on any system, and none is a file
    ///   where another needs a directory.
    /// - No file is over [`FILE_MAX`], and all of them together are not over [`BUNDLE_MAX`].
    /// - Nothing looks like a credential.
    /// - A skill has a `SKILL.md` at its top whose frontmatter names the item and describes
    ///   it. A command or a subagent is the one file `<name>.md` with a description in its
    ///   frontmatter, and a subagent's frontmatter names it too.
    pub fn check(kind: Kind, name: &str, mut files: Vec<File>) -> Result<Bundle, Refused> {
        check_name(name)?;
        if files.is_empty() {
            return Err(Refused(format!("the {kind} {name} holds no files")));
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut seen = HashSet::new();
        let mut total = 0;
        for file in &files {
            check_path(&file.path)?;
            if !seen.insert(file.path.as_str()) {
                return Err(Refused(format!("{} appears twice", file.path)));
            }
            if file.bytes.len() > FILE_MAX {
                return Err(too_big(&file.path, file.bytes.len()));
            }
            total += file.bytes.len();
        }
        for file in &files {
            if let Some(parent) = file.path.match_indices('/').map(|(at, _)| &file.path[..at]).find(|p| seen.contains(p)) {
                return Err(Refused(format!(
                    "{parent} is a file, and {} needs it to be a directory",
                    file.path
                )));
            }
        }
        if total > BUNDLE_MAX {
            return Err(Refused(format!(
                "the {kind} {name} is {} in all, over the {} one version may hold",
                in_bytes(total),
                mib(BUNDLE_MAX)
            )));
        }
        find_credentials(&files)?;
        let description = match kind {
            Kind::Skill => {
                let Some(skill) = files.iter().find(|f| f.path == "SKILL.md") else {
                    return Err(Refused(format!("a skill needs a SKILL.md at its top, and {name} has none")));
                };
                let fields = frontmatter(skill)?;
                same_name(&fields, name, skill)?;
                described(&fields, skill)?
            }
            Kind::Command | Kind::Subagent => {
                let want = format!("{name}.md");
                if files.len() != 1 || files[0].path != want {
                    return Err(Refused(format!("a {kind} is one Markdown file, {want}, and nothing beside it")));
                }
                let fields = frontmatter(&files[0])?;
                if kind == Kind::Subagent {
                    same_name(&fields, name, &files[0])?;
                }
                described(&fields, &files[0])?
            }
        };
        Ok(Bundle { kind, name: name.to_string(), files, description })
    }

    /// Read a version from where it sits on a disk: a skill's directory, or the one file of a
    /// command or a subagent, which the bundle names `<name>.md` whatever it is called there.
    /// A link inside a skill's directory is refused rather than followed, and [`LEFT_OUT`]
    /// clutter is skipped. A file over [`FILE_MAX`] is refused before it is read.
    pub fn from_path(kind: Kind, name: &str, path: &Path) -> Result<Bundle, Refused> {
        let mut files = Vec::new();
        match kind {
            Kind::Skill => walk(path, "", &mut files)?,
            Kind::Command | Kind::Subagent => {
                let meta = std::fs::metadata(path).map_err(|e| unreadable(path, e))?;
                if !meta.is_file() {
                    return Err(Refused(format!("{} is not a file: a {kind} is one Markdown file", path.display())));
                }
                files.push(File { path: format!("{name}.md"), bytes: read(path, meta.len())?, executable: false });
            }
        }
        Bundle::check(kind, name, files)
    }

    /// Read a version back from its canonical form, or from any tar archive of plain files.
    pub fn unpack(kind: Kind, name: &str, archive: &[u8]) -> Result<Bundle, Refused> {
        Bundle::check(kind, name, read_tar(archive)?)
    }

    /// Read an upload, by what its first bytes say it is: a gzipped tar, a zip (as Finder,
    /// `zip` and claude.ai's skill downloads make), a plain tar, or else one Markdown file —
    /// a skill's `SKILL.md`, or a command's or subagent's one file. What a laptop's archive
    /// carries besides the item is dropped first: a folder around everything, a machine's
    /// clutter ([`LEFT_OUT`]), and macOS's `__MACOSX` folder and `._` companions. A command's
    /// or subagent's one file takes the item's name, as [`Bundle::from_path`] gives it.
    pub fn from_upload(kind: Kind, name: &str, upload: &[u8]) -> Result<Bundle, Refused> {
        let files = if upload.starts_with(&[0x1f, 0x8b]) {
            read_tar(&gunzip(upload)?)?
        } else if upload.starts_with(b"PK\x03\x04") {
            read_zip(upload)?
        } else if upload.get(257..262) == Some(&b"ustar"[..]) {
            read_tar(upload)?
        } else if std::str::from_utf8(upload).is_ok() {
            let path = if kind == Kind::Skill { "SKILL.md".to_string() } else { format!("{name}.md") };
            vec![File { path, bytes: upload.to_vec(), executable: false }]
        } else {
            return Err(Refused("this upload is not a .tar.gz, a .zip, a .tar, nor a Markdown file".into()));
        };
        Bundle::check(kind, name, tidy(kind, name, files))
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn files(&self) -> &[File] {
        &self.files
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    /// The canonical form: see the module's comment. Two zero blocks end it, as tar expects.
    pub fn pack(&self) -> Vec<u8> {
        let held: usize = self.files.iter().map(|f| f.bytes.len()).sum();
        let mut out = Vec::with_capacity(held + 1024 * (self.files.len() + 2));
        for file in &self.files {
            let mode = if file.executable { 0o755 } else { 0o644 };
            out.extend_from_slice(&header(&file.path, file.bytes.len(), mode, b'0'));
            out.extend_from_slice(&file.bytes);
            out.resize(out.len().next_multiple_of(512), 0);
        }
        out.resize(out.len() + 1024, 0);
        out
    }
}

/// The digest a packed bundle is known by: the sha256 of its bytes, in lowercase hex.
pub fn digest(packed: &[u8]) -> String {
    Sha256::digest(packed).iter().map(|b| format!("{b:02x}")).collect()
}

/// A path a machine can unpack under the item's own directory and nowhere else, on any
/// system: relative and `/`-separated, with no empty, `.` or `..` part, nothing Windows reads
/// specially (`\`, `:`), no control character, and short enough for the archive to hold.
fn check_path(path: &str) -> Result<(), Refused> {
    if path.starts_with('/') || path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(Refused(format!(
            "{path} is not a path inside the item: a bundle's paths are relative, with no empty, . or .. part"
        )));
    }
    if path.chars().any(|c| c == '\\' || c == ':' || c.is_control()) {
        return Err(Refused(format!(
            "{path} holds a character some machines cannot keep in a file name: \\, : or a control character"
        )));
    }
    if split_path(path).is_none() {
        return Err(Refused(format!(
            "{path} is too long to pack: keep each name under 100 characters and the whole path under 255"
        )));
    }
    Ok(())
}

/// Where ustar keeps a path: whole in its 100-byte name field, or split at a `/` between
/// its 155-byte prefix and the name.
fn split_path(path: &str) -> Option<(&str, &str)> {
    if path.len() <= 100 {
        return Some(("", path));
    }
    path.match_indices('/')
        .map(|(at, _)| at)
        .find(|&at| at <= 155 && path.len() - at - 1 <= 100)
        .map(|at| (&path[..at], &path[at + 1..]))
}

/// One ustar header: `path`, `size` bytes, `mode`, and the entry's type (`b'0'` for a file).
/// Owner, group and time are 0, so nothing about the machine that packed it is in it.
fn header(path: &str, size: usize, mode: u32, typeflag: u8) -> [u8; 512] {
    let (prefix, name) = split_path(path).expect("check() keeps only paths that pack");
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    octal(&mut h[100..108], u64::from(mode));
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size as u64);
    octal(&mut h[136..148], 0);
    h[156] = typeflag;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    h[148..156].fill(b' ');
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

/// A number as tar writes it: zero-padded octal digits, then a NUL.
fn octal(field: &mut [u8], value: u64) {
    let digits = format!("{value:0width$o}", width = field.len() - 1);
    field[..digits.len()].copy_from_slice(digits.as_bytes());
    field[digits.len()] = 0;
}

/// Every regular file in a tar archive: the canonical form, and the plain archives `tar`
/// writes. Directories are implied by their files and skipped; so are a pax header's
/// attributes, all but the long `path` one. A link of either kind, a device, or anything else
/// that is not a file is refused: nothing a machine unpacks may point outside the item.
pub fn read_tar(archive: &[u8]) -> Result<Vec<File>, Refused> {
    read_tar_where(archive, |path| Some(path.to_string()))
}

/// The regular files of a tar archive that `keep` gives a path for, under that path. An entry
/// `keep` passes over is skipped whatever it is — a link elsewhere in a repository's archive
/// costs nothing — while one it keeps that is not a plain file is refused, as in [`read_tar`].
pub fn read_tar_where(archive: &[u8], keep: impl Fn(&str) -> Option<String>) -> Result<Vec<File>, Refused> {
    let ended = || Refused("the archive ends in the middle of an entry".into());
    let mut files = Vec::new();
    let mut at = 0;
    let mut long_path: Option<String> = None;
    loop {
        let header = archive.get(at..at + 512).ok_or_else(ended)?;
        if header.iter().all(|&b| b == 0) {
            return Ok(files);
        }
        if !checksum_holds(header) {
            return Err(Refused("this is not a tar archive, or it is damaged: an entry's checksum does not match".into()));
        }
        let size = number(&header[124..136])?;
        let start = at + 512;
        let body = start.checked_add(size).and_then(|end| archive.get(start..end)).ok_or_else(ended)?;
        at = start + size.next_multiple_of(512);
        match header[156] {
            b'x' => {
                long_path = pax_path(body);
                continue;
            }
            b'L' => {
                long_path = Some(text(body)?);
                continue;
            }
            b'g' => continue,
            _ => {}
        }
        let path = match long_path.take() {
            Some(path) => path,
            None => header_path(header)?,
        };
        let path = path.trim_start_matches("./").to_string();
        if header[156] == b'5' {
            continue;
        }
        let Some(path) = keep(&path) else { continue };
        match header[156] {
            b'0' | b'7' | 0 => {
                let executable = number(&header[100..108])? & 0o111 != 0;
                files.push(File { path, bytes: body.to_vec(), executable });
            }
            b'5' => {}
            b'1' | b'2' => return Err(Refused(format!("{path} is a link; a bundle holds only files"))),
            _ => return Err(Refused(format!("{path} is neither a file nor a directory; a bundle holds only files"))),
        }
    }
}

/// The most an upload may unpack to: twice what a version may hold, leaving room for the
/// archive's own headers, and no room for an archive made to fill the router's memory.
const UNPACKED_MAX: usize = 2 * BUNDLE_MAX;

fn gunzip(upload: &[u8]) -> Result<Vec<u8>, Refused> {
    gunzip_at_most(upload, UNPACKED_MAX)
}

/// A gzip stream unpacked, refused once it unpacks to more than `most`.
pub fn gunzip_at_most(gz: &[u8], most: usize) -> Result<Vec<u8>, Refused> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(gz)
        .take(most as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| Refused(format!("this .tar.gz cannot be read: {e}")))?;
    if out.len() > most {
        return Err(Refused(format!("this archive unpacks to more than {}", mib(most))));
    }
    Ok(out)
}

/// Every regular file in a zip archive, stored or deflated. An encrypted entry, a link and a
/// zip64 archive are refused, and so is an entry that does not unpack to what the archive's
/// directory says it holds.
fn read_zip(zip: &[u8]) -> Result<Vec<File>, Refused> {
    use std::io::Read;
    let damaged = || Refused("this .zip is damaged: its directory does not match its entries".into());
    let u16_at = |at: usize| zip.get(at..at + 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]]))).ok_or_else(damaged);
    let u32_at = |at: usize| zip.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or_else(damaged);

    // The directory's end record, searched for from the end, past a comment of any length.
    let last = zip.len().checked_sub(22).ok_or_else(damaged)?;
    let end = (last.saturating_sub(65_535)..=last).rev().find(|&at| zip[at..].starts_with(b"PK\x05\x06")).ok_or_else(damaged)?;
    let count = u16_at(end + 10)?;
    let mut at = u32_at(end + 16)? as usize;
    if count == 0xFFFF || at == 0xFFFF_FFFF {
        return Err(Refused("this .zip is a zip64 archive, which is not read here: upload a .tar.gz".into()));
    }
    let mut files = Vec::new();
    let mut total = 0;
    for _ in 0..count {
        if !zip.get(at..).is_some_and(|rest| rest.starts_with(b"PK\x01\x02")) {
            return Err(damaged());
        }
        let unix = u16_at(at + 4)? >> 8 == 3;
        let flags = u16_at(at + 8)?;
        let method = u16_at(at + 10)?;
        let crc = u32_at(at + 16)?;
        let packed = u32_at(at + 20)? as usize;
        let size = u32_at(at + 24)? as usize;
        let name_len = u16_at(at + 28)?;
        let mode = u32_at(at + 38)? >> 16;
        let local = u32_at(at + 42)? as usize;
        let name = zip.get(at + 46..at + 46 + name_len).ok_or_else(damaged)?;
        let path = String::from_utf8(name.to_vec())
            .map_err(|_| Refused("the .zip names a file in something other than UTF-8".into()))?
            .trim_start_matches("./")
            .to_string();
        at += 46 + name_len + u16_at(at + 30)? + u16_at(at + 32)?;
        if path.ends_with('/') {
            continue;
        }
        if unix && mode & 0o170000 == 0o120000 {
            return Err(Refused(format!("{path} is a link; a bundle holds only files")));
        }
        if flags & 1 != 0 {
            return Err(Refused(format!("{path} is encrypted: upload the files as they are")));
        }
        if size > FILE_MAX {
            return Err(too_big(&path, size));
        }
        total += size;
        if total > BUNDLE_MAX {
            return Err(Refused(format!("this .zip unpacks to more than the {} one version may hold", mib(BUNDLE_MAX))));
        }
        // The entry's own header says where its data starts.
        if !zip.get(local..).is_some_and(|rest| rest.starts_with(b"PK\x03\x04")) {
            return Err(damaged());
        }
        let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
        let data = start.checked_add(packed).and_then(|stop| zip.get(start..stop)).ok_or_else(damaged)?;
        let bytes = match method {
            0 => data.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(size);
                flate2::read::DeflateDecoder::new(data)
                    .take(size as u64 + 1)
                    .read_to_end(&mut out)
                    .map_err(|_| damaged())?;
                out
            }
            _ => {
                return Err(Refused(format!(
                    "{path} is compressed in a way this does not read (method {method}): use deflate, or upload a .tar.gz"
                )))
            }
        };
        let mut sum = flate2::Crc::new();
        sum.update(&bytes);
        if bytes.len() != size || sum.sum() != crc {
            return Err(damaged());
        }
        files.push(File { path, bytes, executable: unix && mode & 0o111 != 0 });
    }
    Ok(files)
}

/// An uploaded archive's files, without what a laptop adds around an item: see
/// [`Bundle::from_upload`].
fn tidy(kind: Kind, name: &str, files: Vec<File>) -> Vec<File> {
    let mut files: Vec<File> = files
        .into_iter()
        .filter(|f| !f.path.split('/').any(|part| LEFT_OUT.contains(&part) || part == "__MACOSX" || part.starts_with("._")))
        .collect();
    let around = files.first().and_then(|f| f.path.split_once('/')).map(|(top, _)| format!("{top}/"));
    if let Some(around) = around.filter(|top| files.iter().all(|f| f.path.starts_with(top.as_str()))) {
        for file in &mut files {
            file.path = file.path[around.len()..].to_string();
        }
    }
    if kind != Kind::Skill && files.len() == 1 && files[0].path.ends_with(".md") {
        files[0].path = format!("{name}.md");
    }
    files
}

/// Archives as other tools write them, for the tests that take uploads: a tar of `files` in
/// the order given, and a deflated zip made on a Unix machine.
#[cfg(test)]
pub(crate) mod made {
    use super::*;

    pub(crate) fn tar(files: &[File]) -> Vec<u8> {
        let mut out = Vec::new();
        for file in files {
            let mode = if file.executable { 0o755 } else { 0o644 };
            out.extend_from_slice(&header(&file.path, file.bytes.len(), mode, b'0'));
            out.extend_from_slice(&file.bytes);
            out.resize(out.len().next_multiple_of(512), 0);
        }
        out.resize(out.len() + 1024, 0);
        out
    }

    pub(crate) fn tar_gz(files: &[File]) -> Vec<u8> {
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&tar(files)).unwrap();
        gz.finish().unwrap()
    }

    pub(crate) fn zip(files: &[File]) -> Vec<u8> {
        use std::io::Write;
        let (mut out, mut directory) = (Vec::new(), Vec::new());
        for file in files {
            let mut deflate = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            deflate.write_all(&file.bytes).unwrap();
            let data = deflate.finish().unwrap();
            let mut crc = flate2::Crc::new();
            crc.update(&file.bytes);
            let name = file.path.as_bytes();
            let offset = out.len() as u32;
            let sizes = [crc.sum(), data.len() as u32, file.bytes.len() as u32];
            out.extend_from_slice(b"PK\x03\x04\x14\0\0\0\x08\0\0\0\0\0");
            sizes.iter().for_each(|n| out.extend_from_slice(&n.to_le_bytes()));
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(&data);
            let mode: u32 = if file.executable { 0o100755 } else { 0o100644 };
            directory.extend_from_slice(b"PK\x01\x02\x14\x03\x14\0\0\0\x08\0\0\0\0\0");
            sizes.iter().for_each(|n| directory.extend_from_slice(&n.to_le_bytes()));
            directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
            directory.extend_from_slice(&[0; 8]);
            directory.extend_from_slice(&(mode << 16).to_le_bytes());
            directory.extend_from_slice(&offset.to_le_bytes());
            directory.extend_from_slice(name);
        }
        let at = out.len() as u32;
        out.extend_from_slice(&directory);
        out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }
}

fn checksum_holds(header: &[u8]) -> bool {
    let sum: u32 = header
        .iter()
        .enumerate()
        .map(|(i, &b)| if (148..156).contains(&i) { u32::from(b' ') } else { u32::from(b) })
        .sum();
    number(&header[148..156]).is_ok_and(|stored| stored == sum as usize)
}

/// A tar number field: octal digits, padded with spaces or NULs on either side.
fn number(field: &[u8]) -> Result<usize, Refused> {
    let digits = std::str::from_utf8(field)
        .map(|s| s.trim_matches(|c: char| c == '\0' || c == ' '))
        .map_err(|_| Refused("this is not a tar archive this can read: a number field is not octal".into()))?;
    if digits.is_empty() {
        return Ok(0);
    }
    usize::from_str_radix(digits, 8)
        .map_err(|_| Refused("this is not a tar archive this can read: a number field is not octal".into()))
}

fn header_path(header: &[u8]) -> Result<String, Refused> {
    let name = text(&header[0..100])?;
    let prefix = if &header[257..262] == b"ustar" { text(&header[345..500])? } else { String::new() };
    Ok(if prefix.is_empty() { name } else { format!("{prefix}/{name}") })
}

/// A NUL-terminated field, which must be UTF-8: a name a machine could not write is refused.
fn text(field: &[u8]) -> Result<String, Refused> {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8(field[..end].to_vec()).map_err(|_| Refused("the archive names a file in something other than UTF-8".into()))
}

/// The `path` a pax extended header gives the entry after it, if it gives one. Each record
/// is `<length> <key>=<value>\n`, the length counting the whole record.
fn pax_path(body: &[u8]) -> Option<String> {
    let mut rest = body;
    let mut path = None;
    while !rest.is_empty() && rest[0] != 0 {
        let space = rest.iter().position(|&b| b == b' ')?;
        let len: usize = std::str::from_utf8(&rest[..space]).ok()?.parse().ok()?;
        let record = rest.get(space + 1..len)?;
        let record = record.strip_suffix(b"\n").unwrap_or(record);
        if let Some(value) = record.strip_prefix(b"path=") {
            path = Some(String::from_utf8(value.to_vec()).ok()?);
        }
        rest = rest.get(len..)?;
    }
    path
}

fn walk(dir: &Path, prefix: &str, files: &mut Vec<File>) -> Result<(), Refused> {
    for entry in std::fs::read_dir(dir).map_err(|e| unreadable(dir, e))? {
        let entry = entry.map_err(|e| unreadable(dir, e))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return Err(Refused(format!("{} holds a file whose name is not UTF-8", dir.display())));
        };
        if LEFT_OUT.contains(&name) {
            continue;
        }
        let rel = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
        let path = entry.path();
        let meta = path.symlink_metadata().map_err(|e| unreadable(&path, e))?;
        if meta.file_type().is_symlink() {
            return Err(Refused(format!(
                "{rel} is a link; a bundle holds only files: copy what it points to in, or leave it out"
            )));
        } else if meta.is_dir() {
            walk(&path, &rel, files)?;
        } else if meta.is_file() {
            if meta.len() > FILE_MAX as u64 {
                return Err(too_big(&rel, meta.len() as usize));
            }
            files.push(File { path: rel, bytes: read(&path, meta.len())?, executable: executable(&meta) });
        } else {
            return Err(Refused(format!("{rel} is neither a file nor a directory; a bundle holds only files")));
        }
    }
    Ok(())
}

fn read(path: &Path, len: u64) -> Result<Vec<u8>, Refused> {
    if len > FILE_MAX as u64 {
        return Err(too_big(&path.display().to_string(), len as usize));
    }
    std::fs::read(path).map_err(|e| unreadable(path, e))
}

#[cfg(unix)]
fn executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_: &std::fs::Metadata) -> bool {
    false
}

fn unreadable(path: &Path, e: std::io::Error) -> Refused {
    Refused(format!("{} cannot be read: {e}", path.display()))
}

fn too_big(path: &str, len: usize) -> Refused {
    Refused(format!("{path} is {}, over the {} one file may hold", in_bytes(len), mib(FILE_MAX)))
}

/// A size exactly, with its thousands marked: `1,048,577 bytes`.
fn in_bytes(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{out} bytes")
}

fn mib(n: usize) -> String {
    format!("{} MiB", n / (1024 * 1024))
}

/// The shapes credentials take, each with the run of token characters that must follow its
/// prefix for it to be one: a mention of `glpat-` in prose is not a token.
const TOKENS: [(&str, usize, &str); 8] = [
    ("sk-ant-", 20, "an Anthropic key"),
    ("glpat-", 20, "a GitLab token"),
    ("github_pat_", 22, "a GitHub token"),
    ("ghp_", 30, "a GitHub token"),
    ("gho_", 30, "a GitHub token"),
    ("ghu_", 30, "a GitHub token"),
    ("ghs_", 30, "a GitHub token"),
    ("ghr_", 30, "a GitHub token"),
];

/// Refuse a bundle that carries a credential, naming the file and the line. Credentials
/// belong in the vault: a file in the Library is copied to every machine and kept, in every
/// version, for good.
fn find_credentials(files: &[File]) -> Result<(), Refused> {
    for file in files {
        let text = String::from_utf8_lossy(&file.bytes);
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if let Some(what) = credential(line, lines.get(i + 1).copied()) {
                return Err(Refused(format!(
                    "{} line {} holds what looks like {what}. Credentials belong in the vault, never in \
                     something the Library copies to every machine",
                    file.path,
                    i + 1
                )));
            }
        }
    }
    Ok(())
}

fn credential(line: &str, next: Option<&str>) -> Option<&'static str> {
    let marker = line.trim();
    if marker.starts_with("-----BEGIN ")
        && (marker.ends_with("PRIVATE KEY-----") || marker.ends_with("PRIVATE KEY BLOCK-----"))
    {
        // Code and documentation name a key by its first line alone. A key is that line with
        // its body under it: base64, or the headers an encrypted key opens with.
        let body = next.map(str::trim).unwrap_or("");
        let base64 = body.len() >= 40 && body.bytes().all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b));
        if base64 || body.contains(": ") {
            return Some("a private key");
        }
    }
    for (prefix, least, what) in TOKENS {
        for (at, _) in line.match_indices(prefix) {
            let run = line[at + prefix.len()..]
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
                .count();
            if run >= least {
                return Some(what);
            }
        }
    }
    None
}

/// The top-level `key: value` pairs of a Markdown file's frontmatter: the block between its
/// first line, `---`, and the next `---`. A value may be plain, quoted, or a `|` or `>`
/// block, and a plain value may run on over indented lines, as YAML allows. Nested keys are
/// folded into their parent's value: nothing here reads them.
fn frontmatter(file: &File) -> Result<Vec<(String, String)>, Refused> {
    let path = &file.path;
    let text = std::str::from_utf8(&file.bytes).map_err(|_| Refused(format!("{path} is not UTF-8 text")))?;
    let mut lines = text.strip_prefix('\u{feff}').unwrap_or(text).lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(Refused(format!(
            "{path} has no frontmatter: its first line must be ---, with its name and description below"
        )));
    }
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut literal = false;
    for line in lines {
        if line.trim_end() == "---" {
            return Ok(fields);
        }
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = fields.last_mut() {
                let piece = line.trim();
                if !piece.is_empty() {
                    if !value.is_empty() {
                        value.push(if literal { '\n' } else { ' ' });
                    }
                    value.push_str(piece);
                }
            }
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        let block = matches!(value, "|" | "|-" | "|+" | ">" | ">-" | ">+");
        literal = block && value.starts_with('|');
        fields.push((key.trim().to_string(), if block { String::new() } else { unquote(value) }));
    }
    Err(Refused(format!("{path}'s frontmatter never ends: it needs a closing line of ---")))
}

fn unquote(value: &str) -> String {
    let quoted = |q: char| value.len() >= 2 && value.starts_with(q) && value.ends_with(q);
    if quoted('"') {
        value[1..value.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
    } else if quoted('\'') {
        value[1..value.len() - 1].replace("''", "'")
    } else {
        value.to_string()
    }
}

fn field<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn same_name(fields: &[(String, String)], name: &str, file: &File) -> Result<(), Refused> {
    match field(fields, "name") {
        Some(given) if given == name => Ok(()),
        Some(given) => Err(Refused(format!(
            "{} names it \"{given}\", but the item is {name}: the two must agree",
            file.path
        ))),
        None => Err(Refused(format!("{}'s frontmatter has no name; it must say name: {name}", file.path))),
    }
}

fn described(fields: &[(String, String)], file: &File) -> Result<String, Refused> {
    match field(fields, "description").map(str::trim) {
        Some(description) if !description.is_empty() => Ok(description.to_string()),
        _ => Err(Refused(format!(
            "{}'s frontmatter has no description: Claude Code lists the item by it, and decides from it \
             when to use it",
            file.path
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, text: &str) -> File {
        File { path: path.into(), bytes: text.as_bytes().to_vec(), executable: false }
    }

    /// Six lines: the frontmatter, a blank line, a heading.
    fn skill_md(name: &str) -> String {
        format!("---\nname: {name}\ndescription: Does one thing well.\n---\n\n# {name}\n")
    }

    fn refused(result: Result<Bundle, Refused>) -> String {
        result.expect_err("should have been refused").0
    }

    fn scratch(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ar-library-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_skill_needs_a_skill_md_naming_itself_with_a_description() {
        let ok = Bundle::check(Kind::Skill, "bench", vec![file("SKILL.md", &skill_md("bench")), file("scripts/run.sh", "echo hi\n")])
            .unwrap();
        assert_eq!(ok.description(), "Does one thing well.");

        let why = refused(Bundle::check(Kind::Skill, "bench", vec![file("README.md", "# bench\n")]));
        assert!(why.contains("needs a SKILL.md at its top"), "{why}");
        let why = refused(Bundle::check(Kind::Skill, "bench", vec![file("bench/SKILL.md", &skill_md("bench"))]));
        assert!(why.contains("needs a SKILL.md at its top"), "{why}");
        let why = refused(Bundle::check(Kind::Skill, "bench", vec![file("SKILL.md", &skill_md("vault"))]));
        assert!(why.contains("\"vault\"") && why.contains("the item is bench"), "{why}");
        let why = refused(Bundle::check(Kind::Skill, "bench", vec![file("SKILL.md", "---\ndescription: x\n---\n")]));
        assert!(why.contains("has no name"), "{why}");
        let why = refused(Bundle::check(Kind::Skill, "bench", vec![file("SKILL.md", "---\nname: bench\n---\n")]));
        assert!(why.contains("has no description"), "{why}");
        let why = refused(Bundle::check(Kind::Skill, "bench", vec![file("SKILL.md", "# bench\n")]));
        assert!(why.contains("has no frontmatter"), "{why}");
        let why = refused(Bundle::check(Kind::Skill, "bench", vec![file("SKILL.md", "---\nname: bench\ndescription: x\n")]));
        assert!(why.contains("never ends"), "{why}");
        let why = refused(Bundle::check(Kind::Skill, "Bench", vec![file("SKILL.md", &skill_md("Bench"))]));
        assert!(why.contains("cannot name an item"), "{why}");

        // The forms a description takes in real skills.
        for (text, want) in [
            ("---\nname: bench\ndescription: \"Says \\\"hi\\\": quoted.\"\n---\n", "Says \"hi\": quoted."),
            ("---\nname: bench\ndescription: >\n  Folded over\n  two lines.\n---\n", "Folded over two lines."),
            ("---\nname: bench\ndescription: Runs on\n  over an indented line.\n---\n", "Runs on over an indented line."),
            ("---\nname: 'bench'\ndescription: 'It''s quoted.'\nlicense: MIT\n---\n", "It's quoted."),
        ] {
            let bundle = Bundle::check(Kind::Skill, "bench", vec![file("SKILL.md", text)]).expect(text);
            assert_eq!(bundle.description(), want);
        }
    }

    #[test]
    fn a_command_or_subagent_is_one_markdown_file_with_frontmatter() {
        let command = "---\ndescription: Ship the current branch.\n---\nRun the release.\n";
        let ok = Bundle::check(Kind::Command, "ship", vec![file("ship.md", command)]).unwrap();
        assert_eq!(ok.description(), "Ship the current branch.");
        let why = refused(Bundle::check(Kind::Command, "ship", vec![file("ship.md", command), file("notes.md", "n\n")]));
        assert!(why.contains("one Markdown file, ship.md"), "{why}");
        let why = refused(Bundle::check(Kind::Command, "ship", vec![file("release.md", command)]));
        assert!(why.contains("one Markdown file, ship.md"), "{why}");
        let why = refused(Bundle::check(Kind::Command, "ship", vec![file("ship.md", "Run the release.\n")]));
        assert!(why.contains("has no frontmatter"), "{why}");

        let subagent = "---\nname: reviewer\ndescription: Reviews a diff.\ntools: Read, Grep\n---\nYou review.\n";
        let ok = Bundle::check(Kind::Subagent, "reviewer", vec![file("reviewer.md", subagent)]).unwrap();
        assert_eq!(ok.description(), "Reviews a diff.");
        let why = refused(Bundle::check(Kind::Subagent, "critic", vec![file("critic.md", subagent)]));
        assert!(why.contains("\"reviewer\"") && why.contains("the item is critic"), "{why}");
        let why = refused(Bundle::check(Kind::Subagent, "critic", vec![file("critic.md", "---\ndescription: x\n---\n")]));
        assert!(why.contains("has no name"), "{why}");

        // Read from disk, the one file takes the item's name whatever it was called there.
        let dir = scratch("command");
        std::fs::write(dir.join("release-notes.md"), command).unwrap();
        let read = Bundle::from_path(Kind::Command, "ship", &dir.join("release-notes.md")).unwrap();
        assert_eq!(read.files()[0].path, "ship.md");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bundle_with_a_link_or_an_escaping_path_is_refused() {
        for path in ["/etc/passwd", "../SKILL.md", "scripts/../../x", "./SKILL.md", "a//b", "scripts/", "C:/x", "a\\b"] {
            let why = refused(Bundle::check(
                Kind::Skill,
                "bench",
                vec![file("SKILL.md", &skill_md("bench")), file(path, "x\n")],
            ));
            assert!(why.contains(path), "{path}: {why}");
        }
        let why = refused(Bundle::check(
            Kind::Skill,
            "bench",
            vec![file("SKILL.md", &skill_md("bench")), file("scripts", "x\n"), file("scripts/run.sh", "y\n")],
        ));
        assert!(why.contains("scripts is a file"), "{why}");

        // A link inside an archive, of either kind.
        let good = Bundle::check(Kind::Skill, "bench", vec![file("SKILL.md", &skill_md("bench"))]).unwrap();
        for typeflag in [b'2', b'1'] {
            let mut link = header("hosts", 0, 0o644, typeflag).to_vec();
            // Naming the link's target changes the header, so its checksum is made again.
            link[157..167].copy_from_slice(b"/etc/hosts");
            link[148..156].fill(b' ');
            let sum: u32 = link.iter().map(|&b| u32::from(b)).sum();
            link[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
            let packed = [link, good.pack()].concat();
            let why = refused(Bundle::unpack(Kind::Skill, "bench", &packed));
            assert!(why.contains("hosts is a link"), "{why}");
        }

        // A link in a skill's directory on disk is refused, not followed.
        #[cfg(unix)]
        {
            let dir = scratch("link");
            std::fs::write(dir.join("SKILL.md"), skill_md("bench")).unwrap();
            std::os::unix::fs::symlink("/etc/hosts", dir.join("hosts")).unwrap();
            let why = refused(Bundle::from_path(Kind::Skill, "bench", &dir));
            assert!(why.contains("hosts is a link"), "{why}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn an_oversized_file_or_bundle_is_refused_in_words() {
        let big = File { path: "data.bin".into(), bytes: vec![b'x'; FILE_MAX + 1], executable: false };
        let why = refused(Bundle::check(Kind::Skill, "bench", vec![file("SKILL.md", &skill_md("bench")), big]));
        assert!(why.contains("data.bin is 1,048,577 bytes, over the 1 MiB one file may hold"), "{why}");

        let mut files = vec![file("SKILL.md", &skill_md("bench"))];
        files.extend((0..9).map(|n| File { path: format!("part{n}.bin"), bytes: vec![b'x'; FILE_MAX], executable: false }));
        let why = refused(Bundle::check(Kind::Skill, "bench", files));
        assert!(why.contains("the skill bench is 9,437,") && why.contains("over the 8 MiB one version may hold"), "{why}");

        // From a disk, a file over the limit is refused by its size, before it is read.
        let dir = scratch("big");
        std::fs::write(dir.join("SKILL.md"), skill_md("bench")).unwrap();
        std::fs::File::create(dir.join("dump.bin")).unwrap().set_len(FILE_MAX as u64 + 1).unwrap();
        let why = refused(Bundle::from_path(Kind::Skill, "bench", &dir));
        assert!(why.contains("dump.bin is 1,048,577 bytes"), "{why}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bundle_carrying_a_credential_is_refused_naming_file_and_line() {
        // Built here rather than written out, so no scanner mistakes this file for a leak.
        let token = |prefix: &str, n: usize| format!("{prefix}{}", "Q7x".repeat(n).chars().take(n).collect::<String>());
        let key = format!("-----BEGIN {} PRIVATE KEY-----\n{}\n", "OPENSSH", "b3Blbn".repeat(12));
        let cases = [
            ("SKILL.md", 7, format!("export GITLAB_TOKEN={}", token("glpat-", 20)), "a GitLab token"),
            ("scripts/call.sh", 2, format!("curl -H 'x-api-key: {}'", token(&format!("sk-{}-", "ant"), 40)), "an Anthropic key"),
            ("notes.md", 1, format!("token {}", token(&format!("gh{}_", "p"), 36)), "a GitHub token"),
            ("keys/deploy", 1, key, "a private key"),
        ];
        for (path, line, text, what) in cases {
            let files = if path == "SKILL.md" {
                vec![file("SKILL.md", &format!("{}{text}\n", skill_md("bench")))]
            } else {
                let lead = if line == 2 { "#!/bin/sh\n" } else { "" };
                vec![file("SKILL.md", &skill_md("bench")), file(path, &format!("{lead}{text}\n"))]
            };
            let why = refused(Bundle::check(Kind::Skill, "bench", files));
            assert!(why.contains(&format!("{path} line {line} holds what looks like {what}")), "{why}");
            assert!(why.contains("vault"), "{why}");
        }

        // Naming a credential is not carrying one.
        let mentions = format!(
            "Tokens start glpat- or ghp_ and then letters.\nconst START: &[u8] = b\"-----BEGIN {k} PRIVATE KEY-----\";\n\
             -----BEGIN {k} PRIVATE KEY-----\n...\n-----END {k} PRIVATE KEY-----\n",
            k = "OPENSSH"
        );
        Bundle::check(Kind::Skill, "bench", vec![file("SKILL.md", &skill_md("bench")), file("notes.md", &mentions)])
            .expect("a mention is not a credential");
    }

    #[test]
    fn the_same_files_make_the_same_digest() {
        let files = vec![file("SKILL.md", &skill_md("bench")), file("scripts/run.sh", "echo hi\n"), file("notes.md", "n\n")];
        let mut reversed = files.clone();
        reversed.reverse();
        let one = Bundle::check(Kind::Skill, "bench", files).unwrap();
        let two = Bundle::check(Kind::Skill, "bench", reversed).unwrap();
        assert_eq!(digest(&one.pack()), digest(&two.pack()), "order does not matter");
        let back = Bundle::unpack(Kind::Skill, "bench", &one.pack()).unwrap();
        assert_eq!(back.files(), one.files(), "the canonical form reads back as it was");

        // On disk: other times, other modes but the executable bit, and a machine's clutter.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let write = |dir: &Path, mode: u32, secs: u64| {
                std::fs::create_dir_all(dir.join("scripts")).unwrap();
                for (path, text) in [("SKILL.md", skill_md("bench")), ("scripts/run.sh", "echo hi\n".into()), ("notes.md", "n\n".into())] {
                    let p = dir.join(path);
                    std::fs::write(&p, text).unwrap();
                    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
                    let when = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
                    std::fs::File::options().write(true).open(&p).unwrap().set_modified(when).unwrap();
                }
            };
            let (a, b) = (scratch("same-a"), scratch("same-b"));
            write(&a, 0o600, 1_000_000);
            write(&b, 0o644, 1_900_000_000);
            std::fs::write(b.join(".DS_Store"), "finder").unwrap();
            std::fs::create_dir_all(b.join("__pycache__")).unwrap();
            std::fs::write(b.join("__pycache__/x.pyc"), "bytecode").unwrap();
            let from_a = Bundle::from_path(Kind::Skill, "bench", &a).unwrap();
            let from_b = Bundle::from_path(Kind::Skill, "bench", &b).unwrap();
            assert_eq!(digest(&from_a.pack()), digest(&from_b.pack()));
            assert_eq!(digest(&from_a.pack()), digest(&one.pack()), "a directory packs as its files do");

            std::fs::set_permissions(b.join("scripts/run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
            let runnable = Bundle::from_path(Kind::Skill, "bench", &b).unwrap();
            assert_ne!(digest(&runnable.pack()), digest(&from_a.pack()), "the executable bit is kept");
            let back = Bundle::unpack(Kind::Skill, "bench", &runnable.pack()).unwrap();
            assert!(back.files().iter().any(|f| f.path == "scripts/run.sh" && f.executable));

            // Any tar reads the canonical form as it is.
            if Path::new("/usr/bin/tar").exists() {
                std::fs::write(a.join("blob.tar"), runnable.pack()).unwrap();
                let listed = std::process::Command::new("/usr/bin/tar").arg("tvf").arg(a.join("blob.tar")).output().unwrap();
                let listed = String::from_utf8_lossy(&listed.stdout);
                assert!(listed.contains("scripts/run.sh") && listed.contains("rwxr-xr-x"), "{listed}");
            }
            let _ = std::fs::remove_dir_all(&a);
            let _ = std::fs::remove_dir_all(&b);
        }
    }
}
