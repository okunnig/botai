//! Local HTTP fixture standing in for the GitHub Releases API and asset downloads.
#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use botai::update::install::sha256_hex;
use botai::update::platform::{self, ArchiveKind};

pub const REPO_PATH: &str = "/repos/okunnig/botai";

/// Path -> (status, body).
pub type Routes = HashMap<String, (u16, Vec<u8>)>;
/// Path -> extra response headers.
pub type Headers = HashMap<String, Vec<(String, String)>>;

/// Serves `Routes` on 127.0.0.1 until the test process exits.
pub struct FixtureServer {
    pub base: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl FixtureServer {
    /// Binds first so `build` can embed the base URL in release JSON.
    pub fn start(build: impl FnOnce(&str) -> Routes) -> Self {
        Self::start_with_headers(|base| (build(base), Headers::new()))
    }

    /// Like [`Self::start`], with extra response headers per path.
    pub fn start_with_headers(build: impl FnOnce(&str) -> (Routes, Headers)) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (routes, headers) = build(&base);
        let fixture = Arc::new((routes, headers));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&requests);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let fixture = Arc::clone(&fixture);
                let log = Arc::clone(&log);
                thread::spawn(move || handle(stream, &fixture.0, &fixture.1, &log));
            }
        });
        Self { base, requests }
    }

    /// Raw request heads received so far.
    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn handle(mut stream: TcpStream, routes: &Routes, headers: &Headers, log: &Mutex<Vec<String>>) {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    log.lock().unwrap().push(head);

    let (status, body) = routes
        .get(&path)
        .or_else(|| routes.get(path.split('?').next().unwrap_or("")))
        .cloned()
        .unwrap_or((404, br#"{"message":"Not Found"}"#.to_vec()));
    let reason = if status == 200 { "OK" } else { "ERR" };
    let extra: String = headers
        .get(path.split('?').next().unwrap_or(""))
        .into_iter()
        .flatten()
        .map(|(k, v)| format!("{k}: {v}\r\n"))
        .collect();
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&body);
    let _ = stream.flush();
}

/// Builds a gzip tarball; names are written raw so tests can include unsafe paths.
pub fn tar_gz<S: AsRef<str>>(entries: &[(S, &[u8])]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut builder = tar::Builder::new(gz);
    for (name, data) in entries {
        let mut header = tar::Header::new_gnu();
        let name_bytes = name.as_ref().as_bytes();
        header.as_gnu_mut().unwrap().name[..name_bytes.len()].copy_from_slice(name_bytes);
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        builder.append(&header, *data).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

/// Builds a zip archive.
pub fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in entries {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(data).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// Archive in this platform's release format; entries are given without a directory prefix.
pub fn platform_archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    match platform::current_asset("v0.0.0").unwrap().kind {
        ArchiveKind::TarGz => {
            let prefixed: Vec<(String, &[u8])> = entries
                .iter()
                .map(|(n, d)| (format!("./{n}"), *d))
                .collect();
            tar_gz(&prefixed)
        }
        ArchiveKind::Zip => zip(entries),
    }
}

/// A release archive containing `binary` plus the README/LICENSE noise of real releases.
pub fn release_archive(binary: &[u8]) -> Vec<u8> {
    platform_archive(&[
        (platform::binary_name(), binary),
        ("README.md", b"readme"),
        ("LICENSE", b"license"),
    ])
}

pub fn asset_name(tag: &str) -> String {
    platform::current_asset(tag).unwrap().name
}

/// Release JSON with download URLs pointing at the fixture.
pub fn release_json(base: &str, tag: &str, assets: &[&str]) -> String {
    let assets: Vec<String> = assets
        .iter()
        .map(|name| {
            format!(r#"{{"name":"{name}","browser_download_url":"{base}/download/{tag}/{name}"}}"#)
        })
        .collect();
    format!(
        r#"{{"tag_name":"{tag}","draft":false,"assets":[{}]}}"#,
        assets.join(",")
    )
}

/// What a fixture release should serve.
pub struct FixtureRelease {
    pub tag: String,
    pub archive: Option<Vec<u8>>,
    /// `None` computes a correct SHA256SUMS for the archive.
    pub sums: Option<String>,
}

impl FixtureRelease {
    pub fn new(tag: &str, binary: &[u8]) -> Self {
        Self {
            tag: tag.to_string(),
            archive: Some(release_archive(binary)),
            sums: None,
        }
    }
}

/// Adds `/releases/tags/<tag>` and the download routes for `release`.
pub fn add_release(routes: &mut Routes, base: &str, release: &FixtureRelease) -> String {
    let tag = &release.tag;
    let archive_name = asset_name(tag);
    let mut assets = vec!["SHA256SUMS"];
    if let Some(archive) = &release.archive {
        assets.push(&archive_name);
        routes.insert(
            format!("/download/{tag}/{archive_name}"),
            (200, archive.clone()),
        );
    }
    let sums = release.sums.clone().unwrap_or_else(|| {
        let hash = release
            .archive
            .as_ref()
            .map(|a| sha256_hex(a))
            .unwrap_or_default();
        format!("{hash}  {archive_name}\n")
    });
    routes.insert(
        format!("/download/{tag}/SHA256SUMS"),
        (200, sums.into_bytes()),
    );
    let json = release_json(base, tag, &assets);
    routes.insert(
        format!("{REPO_PATH}/releases/tags/{tag}"),
        (200, json.clone().into_bytes()),
    );
    json
}

/// Fixture with `latest` as `/releases/latest`, plus every release in `others` by tag and in the list.
pub fn serve(latest: FixtureRelease, others: Vec<FixtureRelease>) -> FixtureServer {
    FixtureServer::start(|base| {
        let mut routes = Routes::new();
        let latest_json = add_release(&mut routes, base, &latest);
        routes.insert(
            format!("{REPO_PATH}/releases/latest"),
            (200, latest_json.clone().into_bytes()),
        );
        let mut list = vec![latest_json];
        for release in &others {
            list.push(add_release(&mut routes, base, release));
        }
        routes.insert(
            format!("{REPO_PATH}/releases"),
            (200, format!("[{}]", list.join(",")).into_bytes()),
        );
        routes
    })
}
