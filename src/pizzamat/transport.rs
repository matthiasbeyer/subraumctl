//! EDI uses `cmd` for commands and `msg` for replies, routed by src with
//! `recv` replaced by `send`. It does not implement AMQP correlation IDs.

use eyre::{Result, ensure, eyre};
use futures_lite::StreamExt;
use lapin::{BasicProperties, Connection, ConnectionProperties, options::*, types::FieldTable};
use serde_json::{Value, json};

use super::{Options, Request};

pub async fn execute(options: &Options, request: &Request, dry_run: bool) -> Result<Value> {
    let command = request.command(options.debug);
    if dry_run {
        return Ok(
            json!({"dry_run": true, "submitted": false, "command": command,
            "user": request.user, "args": request.args}),
        );
    }
    // Errors intentionally omit the address, which can contain credentials.
    let connection = tokio::time::timeout(
        options.timeout,
        Connection::connect(&options.amqp_url, ConnectionProperties::default()),
    )
    .await
    .map_err(|_| eyre!("Pizzamat connection timed out"))?
    .map_err(|_| eyre!("Pizzamat AMQP connection failed"))?;
    let result = tokio::time::timeout(
        options.timeout,
        exchange(&connection, request, options.debug),
    )
    .await
    .map_err(|_| {
        eyre!(
            "Pizzamat reply timed out; command may have been applied; check orders before retrying"
        )
    })
    .and_then(|result| result);
    // Close even on timeout or malformed replies; exclusive queues disappear.
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        connection.close(200, "done"),
    )
    .await;
    result.map(|message| {
        json!({"dry_run": false, "submitted": true,
        "command": command, "message": message})
    })
}

async fn exchange(connection: &Connection, request: &Request, debug: bool) -> Result<String> {
    let channel = connection
        .create_channel()
        .await
        .map_err(|_| eyre!("Pizzamat AMQP channel failed"))?;
    let queue = channel
        .queue_declare(
            "",
            QueueDeclareOptions {
                exclusive: true,
                auto_delete: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
        .map_err(|_| eyre!("Pizzamat reply queue failed"))?;
    // Server-generated queue names are unique even across clients/processes.
    let source = format!("subraumctl.recv.{}", queue.name());
    let destination = source.replace("recv", "send");
    channel
        .queue_bind(
            queue.name().as_str(),
            "msg",
            &destination,
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await
        .map_err(|_| eyre!("Pizzamat reply binding failed (EDI msg exchange required)"))?;
    let mut consumer = channel
        .basic_consume(
            queue.name().as_str(),
            "",
            BasicConsumeOptions {
                no_ack: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
        .map_err(|_| eyre!("Pizzamat reply consumer failed"))?;
    channel
        .confirm_select(ConfirmSelectOptions::default())
        .await
        .map_err(|_| eyre!("Pizzamat publisher confirmation setup failed"))?;
    let payload = serde_json::to_vec(&request.payload(debug, &source))?;
    let confirmation = channel
        .basic_publish(
            "cmd",
            &request.command(debug),
            BasicPublishOptions {
                mandatory: true,
                ..Default::default()
            },
            &payload,
            BasicProperties::default().with_content_type("application/json".into()),
        )
        .await
        .map_err(|_| eyre!("Pizzamat publish failed; command may have been applied"))?
        .await
        .map_err(|_| {
            eyre!("Pizzamat publish confirmation failed; command may have been applied")
        })?;
    ensure!(
        confirmation.is_ack() && confirmation.take_message().is_none(),
        "Pizzamat command was not routed or acknowledged (check server and debug mode)"
    );
    let delivery = consumer
        .next()
        .await
        .ok_or_else(|| eyre!("Pizzamat reply consumer closed; command may have been applied"))?
        .map_err(|_| eyre!("Pizzamat reply failed; command may have been applied"))?;
    decode_reply(&delivery.data, &destination)
}

fn decode_reply(bytes: &[u8], destination: &str) -> Result<String> {
    ensure!(bytes.len() <= 1024 * 1024, "Pizzamat reply exceeds 1 MiB");
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| eyre!("invalid Pizzamat reply JSON"))?;
    ensure!(
        value["user"].as_str() == Some(destination),
        "unexpected Pizzamat reply destination"
    );
    value["msg"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| eyre!("Pizzamat reply has no message"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edi_reply_is_scoped_and_only_message_is_returned() {
        let reply = br#"{"msg":"alice: pizza","user":"subraumctl.send.test","data":null}"#;
        assert_eq!(
            decode_reply(reply, "subraumctl.send.test").unwrap(),
            "alice: pizza"
        );
        assert!(decode_reply(reply, "other").is_err());
        assert!(decode_reply(br#"{"user":"other"}"#, "other").is_err());
        assert!(decode_reply(b"not json", "other").is_err());
    }
}
