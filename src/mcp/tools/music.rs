use std::sync::Arc;

use eyre::Result;
use rmcp::schemars::{self, JsonSchema};
use serde::Deserialize;
use serde_json::json;

use super::super::backend::Backend;
use super::{Access, Empty, Group, McpServer, add, choice::Choice};
use crate::music::{Request, STATIONS, StationId};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Station {
    station: Choice<StationId>,
}

fn add_request<B: Backend>(
    server: &mut McpServer,
    name: &'static str,
    description: &'static str,
    access: Access,
    backend: Arc<B>,
    request: Request,
) -> Result<()> {
    add(
        server,
        name,
        description,
        Group::Music,
        access,
        move |_: Empty, context| {
            let backend = backend.clone();
            let request = request.clone();
            async move { backend.music(request, context).await }
        },
    )
}

pub(super) fn register<B: Backend>(server: &mut McpServer, backend: Arc<B>) -> Result<()> {
    add_request(
        server,
        "music_status",
        "Read the current MPD playback state, title and time.",
        Access::Read,
        backend.clone(),
        Request::Status,
    )?;
    add(
        server,
        "music_stations",
        "List known radio station ids and names.",
        Group::Music,
        Access::Read,
        |_: Empty, _| async {
            Ok(json!({"stations": STATIONS.iter().map(|station| json!({
                "station": station.id, "name": station.name
            })).collect::<Vec<_>>()}))
        },
    )?;
    for (name, description, request) in [
        ("music_play", "Start or resume MPD playback.", Request::Play),
        ("music_pause", "Pause MPD playback.", Request::Pause),
        (
            "music_next",
            "Play the next MPD queue entry.",
            Request::Next,
        ),
        (
            "music_previous",
            "Play the previous MPD queue entry.",
            Request::Previous,
        ),
    ] {
        add_request(
            server,
            name,
            description,
            Access::Write,
            backend.clone(),
            request,
        )?;
    }
    add(
        server,
        "music_station",
        "Replace the MPD queue with a radio station and play it.",
        Group::Music,
        Access::Write,
        move |args: Station, context| {
            let backend = backend.clone();
            async move {
                backend
                    .music(Request::Station(args.station.0), context)
                    .await
            }
        },
    )
}
