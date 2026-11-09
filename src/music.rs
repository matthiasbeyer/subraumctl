//! MPD playback control for the `audiomaster` music host.
//!
//! MPD speaks a small line-oriented protocol on TCP port 6600. Every operation
//! opens one connection, optionally authenticates, sends one or more commands
//! and reads the `OK`/`ACK` terminator. No MPD library is required.

use std::collections::HashMap;
use std::time::Duration;

use clap::ValueEnum;
use clap::builder::PossibleValue;
use eyre::{Result, ensure, eyre};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::time::timeout;

pub const DEFAULT_ADDRESS: &str = "audiomaster:6600";
const MAX_RESPONSE_LINES: usize = 4096;
const MAX_LINE_BYTES: usize = 8192;

#[derive(Clone)]
pub struct Options {
    pub address: String,
    pub password: Option<String>,
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            address: DEFAULT_ADDRESS.to_string(),
            password: None,
            timeout: Duration::from_secs(3),
        }
    }
}

impl Options {
    pub fn from_global(global: &crate::cli::GlobalArgs) -> Self {
        Self {
            address: global.mpd_address.clone(),
            password: global.mpd_password.clone(),
            timeout: Duration::from_secs(global.mpd_timeout),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Station {
    pub id: &'static str,
    pub name: &'static str,
    pub uri: &'static str,
}

/// Static station catalogue. The SomaFM entries are a snapshot of
/// `http://api.somafm.com/channels.xml`; the remaining entries come from the
/// original subraum music actor. MPD resolves `.pls` playlist URLs itself.
pub static STATIONS: &[Station] = &[
    Station {
        id: "7soul",
        name: "Seven Inch Soul",
        uri: "http://somafm.com/7soul130.pls",
    },
    Station {
        id: "beatblender",
        name: "Beat Blender",
        uri: "http://somafm.com/beatblender130.pls",
    },
    Station {
        id: "bootliquor",
        name: "Boot Liquor",
        uri: "http://somafm.com/bootliquor130.pls",
    },
    Station {
        id: "brfm",
        name: "Black Rock FM",
        uri: "http://somafm.com/brfm130.pls",
    },
    Station {
        id: "christmas",
        name: "Christmas Lounge",
        uri: "http://somafm.com/christmas130.pls",
    },
    Station {
        id: "cliqhop",
        name: "cliqhop idm",
        uri: "http://somafm.com/cliqhop130.pls",
    },
    Station {
        id: "covers",
        name: "Covers",
        uri: "http://somafm.com/covers130.pls",
    },
    Station {
        id: "deepspaceone",
        name: "Deep Space One",
        uri: "http://somafm.com/deepspaceone130.pls",
    },
    Station {
        id: "defcon",
        name: "DEF CON Radio",
        uri: "http://somafm.com/defcon130.pls",
    },
    Station {
        id: "digitalis",
        name: "Digitalis",
        uri: "http://somafm.com/digitalis130.pls",
    },
    Station {
        id: "doomed",
        name: "Doomed",
        uri: "http://somafm.com/doomed130.pls",
    },
    Station {
        id: "dronezone",
        name: "Drone Zone",
        uri: "http://somafm.com/dronezone130.pls",
    },
    Station {
        id: "dz2",
        name: "Drone Zone 2",
        uri: "http://somafm.com/dz2130.pls",
    },
    Station {
        id: "dubstep",
        name: "Dub Step Beyond",
        uri: "http://somafm.com/dubstep130.pls",
    },
    Station {
        id: "fluid",
        name: "Fluid",
        uri: "http://somafm.com/fluid130.pls",
    },
    Station {
        id: "folkfwd",
        name: "Folk Forward",
        uri: "http://somafm.com/folkfwd130.pls",
    },
    Station {
        id: "groovesalad",
        name: "Groove Salad",
        uri: "http://somafm.com/groovesalad130.pls",
    },
    Station {
        id: "groovesalad2",
        name: "Groove Salad 2",
        uri: "http://somafm.com/groovesalad2130.pls",
    },
    Station {
        id: "gsclassic",
        name: "Groove Salad Classic",
        uri: "http://somafm.com/gsclassic130.pls",
    },
    Station {
        id: "illstreet",
        name: "Illinois Street Lounge",
        uri: "http://somafm.com/illstreet130.pls",
    },
    Station {
        id: "indiepop",
        name: "Indie Pop Rocks!",
        uri: "http://somafm.com/indiepop130.pls",
    },
    Station {
        id: "jollysoul",
        name: "Jolly Ol' Soul",
        uri: "http://somafm.com/jollysoul130.pls",
    },
    Station {
        id: "live",
        name: "SomaFM Live",
        uri: "http://somafm.com/live130.pls",
    },
    Station {
        id: "lush",
        name: "Lush",
        uri: "http://somafm.com/lush130.pls",
    },
    Station {
        id: "missioncontrol",
        name: "Mission Control",
        uri: "http://somafm.com/missioncontrol130.pls",
    },
    Station {
        id: "poptron",
        name: "PopTron",
        uri: "http://somafm.com/poptron130.pls",
    },
    Station {
        id: "secretagent",
        name: "Secret Agent",
        uri: "http://somafm.com/secretagent130.pls",
    },
    Station {
        id: "seventies",
        name: "Left Coast 70s",
        uri: "http://somafm.com/seventies130.pls",
    },
    Station {
        id: "sf1033",
        name: "SF 10-33",
        uri: "http://somafm.com/sf1033130.pls",
    },
    Station {
        id: "sonicuniverse",
        name: "Sonic Universe",
        uri: "http://somafm.com/sonicuniverse130.pls",
    },
    Station {
        id: "spacestation",
        name: "Space Station Soma",
        uri: "http://somafm.com/spacestation130.pls",
    },
    Station {
        id: "suburbsofgoa",
        name: "Suburbs of Goa",
        uri: "http://somafm.com/suburbsofgoa130.pls",
    },
    Station {
        id: "thetrip",
        name: "The Trip",
        uri: "http://somafm.com/thetrip130.pls",
    },
    Station {
        id: "thistle",
        name: "ThistleRadio",
        uri: "http://somafm.com/thistle130.pls",
    },
    Station {
        id: "u80s",
        name: "Underground 80s",
        uri: "http://somafm.com/u80s130.pls",
    },
    Station {
        id: "xmasinfrisko",
        name: "Xmas in Frisko",
        uri: "http://somafm.com/xmasinfrisko130.pls",
    },
    Station {
        id: "xmasrocks",
        name: "Christmas Rocks!",
        uri: "http://somafm.com/xmasrocks130.pls",
    },
    Station {
        id: "metal",
        name: "Metal Detector",
        uri: "http://somafm.com/metal130.pls",
    },
    Station {
        id: "reggae",
        name: "Heavyweight Reggae",
        uri: "http://somafm.com/reggae130.pls",
    },
    Station {
        id: "scanner",
        name: "SF Police Scanner",
        uri: "http://somafm.com/scanner130.pls",
    },
    Station {
        id: "vaporwaves",
        name: "Vaporwaves",
        uri: "http://somafm.com/vaporwaves130.pls",
    },
    Station {
        id: "specials",
        name: "SomaFM Specials",
        uri: "http://somafm.com/specials130.pls",
    },
    Station {
        id: "n5md",
        name: "n5MD Radio",
        uri: "http://somafm.com/n5md130.pls",
    },
    Station {
        id: "synphaera",
        name: "Synphaera Radio",
        uri: "http://somafm.com/synphaera130.pls",
    },
    Station {
        id: "darkzone",
        name: "The Dark Zone",
        uri: "http://somafm.com/darkzone130.pls",
    },
    Station {
        id: "sfinsf",
        name: "SF in SF",
        uri: "http://somafm.com/sfinsf130.pls",
    },
    Station {
        id: "tikitime",
        name: "Tiki Time",
        uri: "http://somafm.com/tikitime130.pls",
    },
    Station {
        id: "bossa",
        name: "Bossa Beyond",
        uri: "http://somafm.com/bossa130.pls",
    },
    Station {
        id: "insound",
        name: "The In-Sound",
        uri: "http://somafm.com/insound130.pls",
    },
    Station {
        id: "deptstore",
        name: "Department Store Christmas",
        uri: "http://somafm.com/deptstore130.pls",
    },
    Station {
        id: "chillits",
        name: "Chillits Radio",
        uri: "http://somafm.com/chillits130.pls",
    },
    Station {
        id: "bassdrive",
        name: "Bassdrive",
        uri: "http://www.bassdrive.com/v2/streams/BassDrive3.pls",
    },
    Station {
        id: "lunico",
        name: "L'UniCo",
        uri: "http://icecast.l-uni.co:8001/lunico_high_mp3",
    },
    Station {
        id: "technobase",
        name: "TechnoBase.FM",
        uri: "http://listen.technobase.fm/aacplus.pls",
    },
    Station {
        id: "trancebase",
        name: "TranceBase.FM",
        uri: "http://listen.trancebase.fm/aacplus.pls",
    },
    Station {
        id: "teatime",
        name: "TeaTime.FM",
        uri: "http://listen.teatime.fm/aacplus.pls",
    },
    Station {
        id: "equinoxe",
        name: "Radio Equinoxe",
        uri: "http://195.154.185.139/radio/39218/stream/76118",
    },
    Station {
        id: "rockantenne",
        name: "Rockantenne",
        uri: "http://mp3channels.webradio.antenne.de:80/rockantenne",
    },
    Station {
        id: "rock-alt",
        name: "Rockantenne Alternative",
        uri: "http://mp3channels.webradio.rockantenne.de/alternative",
    },
    Station {
        id: "rock-classic",
        name: "Rockantenne Classic Perlen",
        uri: "http://mp3channels.webradio.rockantenne.de/classic-perlen",
    },
    Station {
        id: "cc",
        name: "The Radio.cc",
        uri: "http://theradio.cc:8000/trcc-stream.opus",
    },
    Station {
        id: "nectarine",
        name: "Nectarine Demoscene Radio",
        uri: "http://privat.is-by.us:8000/necta192.mp3",
    },
    Station {
        id: "kohina",
        name: "kohina.ogg",
        uri: "http://kohina.radio.ethz.ch/kohina.ogg",
    },
    Station {
        id: "paranoid",
        name: "Radio Paranoid",
        uri: "http://streamplus31.leonex.de:35194/",
    },
    Station {
        id: "discover_trance",
        name: "Discover Trance",
        uri: "http://paris.discovertrance.com:8006/",
    },
    Station {
        id: "psyndora",
        name: "Psyndora Psytrance",
        uri: "http://cast.magicstreams.gr:9111/stream",
    },
    Station {
        id: "chillout",
        name: "Psyndora Chillout",
        uri: "https://cast.magicstreams.gr/sc/psychill/stream",
    },
    Station {
        id: "ilovemashup",
        name: "I Love Mashup",
        uri: "http://streams.ilovemusic.de/iloveradio5.mp3",
    },
    Station {
        id: "radio",
        name: "R/a/dio",
        uri: "http://r-a-d.io/assets/main.pls",
    },
    Station {
        id: "hardbass",
        name: "Slav Hard Bass Dance 10 Hours",
        uri: "hardbass/Slav Hard Bass Dance 10 Hours.webm",
    },
    Station {
        id: "fluffyradio",
        name: "Pink fluffy unicorns dancing on rainbows",
        uri: "hardbass/pink_fluffy_unicorns_dancing_on_rainbows.mp3",
    },
    Station {
        id: "hardstyle",
        name: "RealHardStyle.nl",
        uri: "https://stream.realhardstyle.nl/",
    },
];

/// Index into [`STATIONS`]. Implements [`ValueEnum`] so CLI and MCP schemas
/// expose the same fixed station list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StationId(usize);

impl StationId {
    const fn new(index: usize) -> Self {
        Self(index)
    }

    pub fn station(self) -> &'static Station {
        &STATIONS[self.0]
    }

    pub fn id(self) -> &'static str {
        self.station().id
    }

    pub fn from_id(id: &str) -> Option<Self> {
        STATIONS
            .iter()
            .position(|station| station.id == id)
            .map(Self)
    }
}

static STATION_IDS: [StationId; STATIONS.len()] = {
    let mut ids = [StationId::new(0); STATIONS.len()];
    let mut index = 0;
    while index < STATIONS.len() {
        ids[index] = StationId::new(index);
        index += 1;
    }
    ids
};

impl ValueEnum for StationId {
    fn value_variants<'a>() -> &'a [Self] {
        &STATION_IDS
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        Some(PossibleValue::new(self.id()).help(self.station().name))
    }
}

impl std::fmt::Display for StationId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.id())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Status,
    Play,
    Pause,
    Next,
    Previous,
    Station(StationId),
}

impl Request {
    pub fn command_preview(&self) -> String {
        match self {
            Self::Status => "status".into(),
            Self::Play => "play".into(),
            Self::Pause => "pause 1".into(),
            Self::Next => "next".into(),
            Self::Previous => "previous".into(),
            Self::Station(station) => format!("station {}", station.id()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaybackStatus {
    pub state: String,
    pub artist: Option<String>,
    pub title: Option<String>,
    pub name: Option<String>,
    pub file: Option<String>,
    pub elapsed: Option<f64>,
    pub duration: Option<f64>,
    pub volume: Option<u8>,
}

impl PlaybackStatus {
    pub fn state_label(&self) -> &str {
        match self.state.as_str() {
            "play" => "Playing",
            "pause" => "Paused",
            "stop" => "Stopped",
            "" => "Unknown",
            other => other,
        }
    }

    pub fn display_title(&self) -> String {
        match (&self.artist, &self.title) {
            (Some(artist), Some(title)) if !artist.is_empty() && !title.is_empty() => {
                format!("{artist} - {title}")
            }
            (_, Some(title)) if !title.is_empty() => title.clone(),
            (_, _) => self
                .name
                .as_ref()
                .filter(|name| !name.is_empty())
                .or_else(|| self.file.as_ref().filter(|file| !file.is_empty()))
                .cloned()
                .unwrap_or_else(|| "Nothing playing".into()),
        }
    }

    pub fn time_label(&self) -> String {
        match (self.elapsed, self.duration) {
            (Some(elapsed), Some(duration)) => {
                format!("{} / {}", format_time(elapsed), format_time(duration))
            }
            (Some(elapsed), None) => format_time(elapsed),
            _ => String::new(),
        }
    }

    pub fn json(&self) -> Value {
        json!({
            "state": self.state,
            "state_label": self.state_label(),
            "display_title": self.display_title(),
            "artist": self.artist,
            "title": self.title,
            "name": self.name,
            "file": self.file,
            "elapsed_seconds": self.elapsed,
            "duration_seconds": self.duration,
            "time": self.time_label(),
            "volume": self.volume,
        })
    }

    fn from_lines(status: &[String], song: &[String]) -> Self {
        let status = key_values(status);
        let song = key_values(song);
        let mut elapsed = status.get("elapsed").and_then(|value| number(value));
        let mut duration = status.get("duration").and_then(|value| number(value));
        if let Some((left, right)) = status.get("time").and_then(|value| value.split_once(':')) {
            elapsed = elapsed.or_else(|| number(left));
            duration = duration.or_else(|| number(right));
        }
        Self {
            state: status.get("state").cloned().unwrap_or_default(),
            artist: text(&song, "Artist"),
            title: text(&song, "Title"),
            name: text(&song, "Name"),
            file: text(&song, "file"),
            elapsed,
            duration,
            volume: status
                .get("volume")
                .and_then(|value| value.trim().parse::<u8>().ok()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Status(PlaybackStatus),
    Submitted,
    DryRun { command: String },
}

impl Outcome {
    pub fn json(&self) -> Value {
        match self {
            Self::Status(status) => status.json(),
            Self::Submitted => json!({"submitted": true}),
            Self::DryRun { command } => {
                json!({"dry_run": true, "submitted": false, "command": command})
            }
        }
    }
}

pub async fn execute(options: &Options, request: &Request, dry_run: bool) -> Result<Outcome> {
    if dry_run {
        return Ok(Outcome::DryRun {
            command: request.command_preview(),
        });
    }
    let mut connection = Connection::connect(options).await?;
    match request {
        Request::Status => Ok(Outcome::Status(connection.status().await?)),
        Request::Play => {
            connection.send("play").await?;
            Ok(Outcome::Submitted)
        }
        Request::Pause => {
            connection.send("pause 1").await?;
            Ok(Outcome::Submitted)
        }
        Request::Next => {
            connection.send("next").await?;
            Ok(Outcome::Submitted)
        }
        Request::Previous => {
            connection.send("previous").await?;
            Ok(Outcome::Submitted)
        }
        Request::Station(id) => {
            let station = id.station();
            ensure!(
                !station.uri.is_empty() && !station.uri.contains(['\n', '\r']),
                "station has no playable URI"
            );
            connection.send("clear").await?;
            connection
                .send(&format!("add {}", quote(station.uri)))
                .await?;
            connection.send("play").await?;
            Ok(Outcome::Submitted)
        }
    }
}

struct Connection {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    timeout: Duration,
}

impl Connection {
    async fn connect(options: &Options) -> Result<Self> {
        ensure!(
            !options.address.is_empty() && !options.address.contains(char::is_whitespace),
            "invalid MPD address"
        );
        let stream = timeout(options.timeout, TcpStream::connect(&options.address))
            .await
            .map_err(|_| eyre!("MPD connection timed out"))?
            .map_err(|_| eyre!("MPD connection failed"))?;
        stream.set_nodelay(true)?;
        let (read, write) = stream.into_split();
        let mut connection = Self {
            reader: BufReader::new(read),
            writer: write,
            timeout: options.timeout,
        };
        let greeting = connection
            .read_line()
            .await?
            .ok_or_else(|| eyre!("MPD closed the connection during the greeting"))?;
        ensure!(greeting.starts_with("OK MPD "), "unexpected MPD greeting");
        if let Some(password) = &options.password {
            ensure!(
                !password.contains(['\n', '\r']),
                "MPD password contains a control character"
            );
            connection
                .send(&format!("password {}", quote(password)))
                .await?;
        }
        Ok(connection)
    }

    async fn status(&mut self) -> Result<PlaybackStatus> {
        let status = self.send("status").await?;
        let song = self.send("currentsong").await?;
        Ok(PlaybackStatus::from_lines(&status, &song))
    }

    async fn send(&mut self, command: &str) -> Result<Vec<String>> {
        ensure!(
            !command.is_empty() && !command.contains(['\n', '\r']),
            "invalid MPD command"
        );
        timeout(
            self.timeout,
            self.writer.write_all(format!("{command}\n").as_bytes()),
        )
        .await
        .map_err(|_| eyre!("MPD write timed out"))??;
        self.read_response().await
    }

    async fn read_response(&mut self) -> Result<Vec<String>> {
        let mut lines = Vec::new();
        loop {
            let line = self
                .read_line()
                .await?
                .ok_or_else(|| eyre!("MPD closed the connection"))?;
            if line == "OK" {
                return Ok(lines);
            }
            if let Some(error) = line.strip_prefix("ACK ") {
                return Err(eyre!("MPD rejected the command: {error}"));
            }
            ensure!(
                lines.len() < MAX_RESPONSE_LINES,
                "MPD response is too large"
            );
            lines.push(line);
        }
    }

    async fn read_line(&mut self) -> Result<Option<String>> {
        let mut line = String::new();
        let read = timeout(self.timeout, self.reader.read_line(&mut line))
            .await
            .map_err(|_| eyre!("MPD read timed out"))??;
        if read == 0 {
            return Ok(None);
        }
        ensure!(line.len() <= MAX_LINE_BYTES, "MPD line is too long");
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        Ok(Some(line))
    }
}

fn key_values(lines: &[String]) -> HashMap<String, String> {
    lines
        .iter()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.to_string(), value.trim_start().to_string()))
        })
        .collect()
}

fn text(values: &HashMap<String, String>, key: &str) -> Option<String> {
    values
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn number(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn format_time(seconds: f64) -> String {
    let seconds = seconds.round() as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn station_ids_are_unique_and_round_trip() {
        let mut seen = std::collections::HashSet::new();
        for id in STATION_IDS {
            assert!(seen.insert(id.id()), "duplicate station id {}", id.id());
            assert_eq!(StationId::from_id(id.id()), Some(id));
        }
        assert_eq!(StationId::from_id("technobase").unwrap().id(), "technobase");
        assert!(StationId::from_id("no-such-station").is_none());
    }

    #[test]
    fn status_decoding_prefers_artist_and_title() {
        let status = vec![
            "volume: 42".to_string(),
            "state: play".to_string(),
            "elapsed: 12.5".to_string(),
            "duration: 125.0".to_string(),
        ];
        let song = vec![
            "file: http://example/stream".to_string(),
            "Name: Groove Salad".to_string(),
            "Artist: Artist".to_string(),
            "Title: Song".to_string(),
        ];
        let decoded = PlaybackStatus::from_lines(&status, &song);
        assert_eq!(decoded.display_title(), "Artist - Song");
        assert_eq!(decoded.time_label(), "0:13 / 2:05");
        assert_eq!(decoded.volume, Some(42));

        let decoded = PlaybackStatus::from_lines(&[], &["Name: Only a station".to_string()]);
        assert_eq!(decoded.display_title(), "Only a station");
        assert_eq!(decoded.time_label(), "");
    }

    #[test]
    fn quotes_spaces_and_backslashes() {
        assert_eq!(quote("simple"), "\"simple\"");
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote("a\\b"), "\"a\\\\b\"");
    }

    #[tokio::test]
    async fn reads_status_and_sends_radio_commands() {
        let (address, server) = fake_server(vec![
            "OK\n",
            "volume: 50\nstate: play\nelapsed: 1.0\nduration: 120.0\nOK\n",
            "file: http://example/stream\nName: Groove Salad\nTitle: Track\nArtist: Artist\nOK\n",
        ])
        .await;
        let options = Options {
            address,
            password: Some("secret".into()),
            timeout: Duration::from_secs(2),
        };
        let status = execute(&options, &Request::Status, false).await.unwrap();
        let Outcome::Status(status) = status else {
            panic!("expected status");
        };
        assert_eq!(status.display_title(), "Artist - Track");
        let commands = server.await.unwrap();
        assert_eq!(
            commands,
            vec!["password \"secret\"", "status", "currentsong"]
        );

        let (address, server) = fake_server(vec!["OK\n", "OK\n", "OK\n"]).await;
        let options = Options {
            address,
            password: None,
            timeout: Duration::from_secs(2),
        };
        let station = StationId::from_id("hardbass").unwrap();
        assert_eq!(
            execute(&options, &Request::Station(station), false)
                .await
                .unwrap(),
            Outcome::Submitted
        );
        assert_eq!(
            server.await.unwrap(),
            vec![
                "clear",
                "add \"hardbass/Slav Hard Bass Dance 10 Hours.webm\"",
                "play"
            ]
        );
    }

    #[tokio::test]
    async fn dry_run_does_not_connect() {
        let options = Options {
            address: "invalid://PRIVATE".into(),
            ..Options::default()
        };
        assert_eq!(
            execute(&options, &Request::Play, true).await.unwrap(),
            Outcome::DryRun {
                command: "play".into()
            }
        );
    }

    async fn fake_server(
        responses: Vec<&'static str>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.write_all(b"OK MPD 0.23.5\n").await.unwrap();
            let (read, mut write) = socket.split();
            let mut reader = BufReader::new(read);
            let mut commands = Vec::new();
            for response in responses {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                commands.push(line.trim_end().to_string());
                write.write_all(response.as_bytes()).await.unwrap();
            }
            commands
        });
        (address, server)
    }
}
