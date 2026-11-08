//! Requires an explicitly configured disposable RabbitMQ; see docs/heizung.md.

use futures_lite::StreamExt;
use lapin::{
    BasicProperties, Connection, ConnectionProperties, ExchangeKind, options::*, types::FieldTable,
};
use serde_json::{Value, json};
use std::time::Duration;
use subraumctl::heizung::{Options, Request, Room, transport};

#[tokio::test]
#[ignore = "requires an explicitly configured disposable AMQP broker"]
async fn edi_round_trip_isolation_failures_and_queue_cleanup() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let address = std::env::var("SUBRAUMCTL_TEST_AMQP_URL")
            .expect("set SUBRAUMCTL_TEST_AMQP_URL to a disposable broker");
        let connection = Connection::connect(&address, ConnectionProperties::default())
            .await
            .unwrap();
        let channel = connection.create_channel().await.unwrap();
        for name in ["cmd", "msg"] {
            channel
                .exchange_declare(
                    name,
                    ExchangeKind::Topic,
                    ExchangeDeclareOptions {
                        durable: true,
                        ..Default::default()
                    },
                    FieldTable::default(),
                )
                .await
                .unwrap();
        }
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
            .unwrap();
        channel
            .queue_bind(
                queue.name().as_str(),
                "cmd",
                "heizung",
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await
            .unwrap();
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
            .unwrap();
        let server_channel = channel.clone();
        let fake_edi = tokio::spawn(async move {
            let mut pending = Vec::new();
            let mut reply_queues = Vec::new();
            for _ in 0..7 {
                let delivery = consumer.next().await.unwrap().unwrap();
                let value: Value = serde_json::from_slice(&delivery.data).unwrap();
                assert_eq!(value["cmd"], "heizung");
                assert_eq!(delivery.routing_key.as_str(), "heizung");
                assert_eq!(value["user"], "subraumctl");
                assert!(value["dst"].is_null());
                let source = value["src"].as_str().unwrap();
                reply_queues.push(source.strip_prefix("subraumctl.recv.").unwrap().to_string());
                let destination = source.replace("recv", "send");
                let reply = |message| {
                    (
                        destination.clone(),
                        // Python edi.emit.msg_reply receives src and user separately:
                        // src determines the AMQP route, user is the original caller.
                        json!({"user": value["user"], "msg": message, "data": "PRIVATE"})
                            .to_string(),
                    )
                };
                let mut replies = Vec::new();
                match value["args"].as_str().unwrap() {
                    "get hack" | "get make" => {
                        let room = value["args"]
                            .as_str()
                            .unwrap()
                            .strip_prefix("get ")
                            .unwrap();
                        pending.push(reply(format!("{room}: 21.5, 02:00:00 remaining")));
                        if pending.len() == 2 {
                            // Same user, different private routes. Reverse the replies
                            // to catch correlation by identity instead of AMQP route.
                            pending.reverse();
                            replies.append(&mut pending);
                        }
                    }
                    "set hack make 21.5 -d 120" => replies.push(reply(
                        "Set temperature in hack,make to 21.5°C for 120 minutes".into(),
                    )),
                    "set hack 22.0 -d 180" => replies.push(reply("Command failed: PRIVATE".into())),
                    "get cook" => replies.push((destination, "not JSON PRIVATE".into())),
                    "get elab" => replies.push(reply("storage: 20.0, 00:00:00 remaining".into())),
                    "set storage 20.0 -d 180" => {} // Applied but reply lost: never retry.
                    other => panic!("unexpected arguments: {other}"),
                }
                for (destination, body) in replies {
                    server_channel
                        .basic_publish(
                            "msg",
                            &destination,
                            BasicPublishOptions::default(),
                            body.as_bytes(),
                            BasicProperties::default(),
                        )
                        .await
                        .unwrap()
                        .await
                        .unwrap();
                }
            }
            reply_queues
        });
        let options = Options {
            amqp_url: address,
            timeout: Duration::from_secs(2),
        };
        let hack = Request::get(vec![Room::Hack]).unwrap();
        let make = Request::get(vec![Room::Make]).unwrap();
        let (a, b) = tokio::join!(
            transport::execute(&options, &hack, false),
            transport::execute(&options, &make, false)
        );
        for (result, room) in [(a, "hack"), (b, "make")] {
            let result = result.unwrap();
            assert_eq!(result["rooms"][0]["room"], room);
            assert_eq!(result["rooms"][0]["remaining_seconds"], 7200);
            assert!(!result.to_string().contains("PRIVATE"));
        }
        let request = Request::set(vec![Room::Hack, Room::Make], 21.5, 120).unwrap();
        assert_eq!(
            transport::execute(&options, &request, false).await.unwrap()["acknowledged"],
            true
        );
        for (request, expected) in [
            (
                Request::set(vec![Room::Hack], 22.0, 180).unwrap(),
                "did not acknowledge",
            ),
            (
                Request::get(vec![Room::Cook]).unwrap(),
                "invalid heizung reply JSON",
            ),
            (
                Request::get(vec![Room::Elab]).unwrap(),
                "unexpected or duplicate",
            ),
            (
                Request::set(vec![Room::Storage], 20.0, 180).unwrap(),
                "may have been applied",
            ),
        ] {
            let error = transport::execute(&options, &request, false)
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("PRIVATE"));
        }
        for queue in fake_edi.await.unwrap() {
            let probe = connection.create_channel().await.unwrap();
            assert!(
                probe
                    .queue_declare(
                        &queue,
                        QueueDeclareOptions {
                            passive: true,
                            ..Default::default()
                        },
                        FieldTable::default()
                    )
                    .await
                    .is_err(),
                "reply queue leaked: {queue}"
            );
        }
        channel
            .queue_unbind(
                queue.name().as_str(),
                "cmd",
                "heizung",
                FieldTable::default(),
            )
            .await
            .unwrap();
        let error = transport::execute(&options, &hack, false)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("not routed"), "{error}");
        connection.close(200, "done").await.unwrap();
    })
    .await
    .unwrap();
}
