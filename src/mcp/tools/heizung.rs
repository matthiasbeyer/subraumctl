use super::*;
use crate::heizung::{DEFAULT_DURATION, Request, Room};
use choice::Choice;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadRooms {
    /// Rooms to read; omitted or empty means all six rooms.
    #[serde(default)]
    rooms: Vec<Choice<Room>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetRooms {
    /// Explicit room selection; required unless all is true.
    #[serde(default)]
    rooms: Vec<Choice<Room>>,
    #[serde(default)]
    all: bool,
    /// Target in Celsius, 15..=25 in 0.1-degree steps.
    #[schemars(range(min = 15, max = 25))]
    temperature: f64,
    /// Minutes before returning to standby; 0 resets the timer.
    #[serde(default = "default_duration")]
    #[schemars(range(min = 0, max = 35791))]
    duration_minutes: u32,
}

fn default_duration() -> u32 {
    DEFAULT_DURATION
}

pub(super) fn register<B: super::super::backend::Backend>(
    server: &mut McpServer,
    backend: Arc<B>,
) -> Result<()> {
    add(
        server,
        "heizung_rooms",
        "List known heating rooms.",
        Group::Heizung,
        Access::Read,
        |_: Empty, _| async {
            Ok(json!({"rooms": Room::ALL.iter().map(|room| room.name()).collect::<Vec<_>>()}))
        },
    )?;
    for name in ["heizung_get", "heizung_status"] {
        let backend = backend.clone();
        add(
            server,
            name,
            "Read heating target temperatures and remaining seconds for selected rooms (all when omitted). Does not report measured temperatures.",
            Group::Heizung,
            Access::Read,
            move |args: ReadRooms, context| {
                let backend = backend.clone();
                let request = Request::get(args.rooms.into_iter().map(|room| room.0).collect());
                async move { backend.heizung(request?, context).await }
            },
        )?;
    }
    add(
        server,
        "heizung_set",
        "Set heating targets for explicit rooms or all rooms, for duration_minutes (default 180). Acknowledgement is not measured physical state.",
        Group::Heizung,
        Access::Write,
        move |args: SetRooms, context| {
            let backend = backend.clone();
            async move {
                eyre::ensure!(
                    !args.all || args.rooms.is_empty(),
                    "select rooms or all, not both"
                );
                let rooms = if args.all {
                    Room::ALL.to_vec()
                } else {
                    args.rooms.into_iter().map(|room| room.0).collect()
                };
                let request = Request::set(rooms, args.temperature, args.duration_minutes)?;
                backend.heizung(request, context).await
            }
        },
    )
}
