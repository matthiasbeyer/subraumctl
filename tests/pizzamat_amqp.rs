//! Run against a disposable RabbitMQ instance, never a production EDI broker:
//! docker run --rm -d --name pizza-test -p 127.0.0.1:25672:5672 rabbitmq:4-alpine
//! SUBRAUMCTL_TEST_AMQP_URL=amqp://127.0.0.1:25672/%2f \
//!   cargo test --test pizzamat_amqp -- --ignored
//! docker stop pizza-test

use futures_lite::StreamExt;
use lapin::{
    BasicProperties, Connection, ConnectionProperties, ExchangeKind, options::*, types::FieldTable,
};
use serde_json::{Value, json};
use std::time::Duration;
use subraumctl::pizzamat::{Operation, Options, Request, transport};

#[tokio::test]
#[ignore = "requires an explicitly configured disposable AMQP broker"]
async fn edi_round_trip_isolation_unrouted_timeout_and_queue_cleanup() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let address = std::env::var("SUBRAUMCTL_TEST_AMQP_URL").expect("set SUBRAUMCTL_TEST_AMQP_URL to a disposable broker");
        let connection = Connection::connect(&address, ConnectionProperties::default()).await.unwrap();
        let channel = connection.create_channel().await.unwrap();
        for name in ["cmd", "msg"] {
            channel.exchange_declare(name, ExchangeKind::Topic,
                ExchangeDeclareOptions { durable: true, ..Default::default() }, FieldTable::default()).await.unwrap();
        }
        let queue = channel.queue_declare("", QueueDeclareOptions {
            exclusive: true, auto_delete: true, ..Default::default()
        }, FieldTable::default()).await.unwrap();
        for command in ["pizza-list", "pizza-add", "pizza-services", "pizza-lock"] {
            channel.queue_bind(queue.name().as_str(), "cmd", command, QueueBindOptions::default(), FieldTable::default()).await.unwrap();
        }
        let mut consumer = channel.basic_consume(queue.name().as_str(), "", BasicConsumeOptions {
            no_ack: true, ..Default::default()
        }, FieldTable::default()).await.unwrap();
        let server_channel = channel.clone();
        let fake_edi = tokio::spawn(async move {
            let mut pending = Vec::new();
            let mut reply_queues = Vec::new();
            for _ in 0..5 {
                let delivery = consumer.next().await.unwrap().unwrap();
                let value: Value = serde_json::from_slice(&delivery.data).unwrap();
                assert_eq!(value["cmd"], delivery.routing_key.as_str());
                assert!(value["dst"].is_null());
                let source = value["src"].as_str().unwrap();
                reply_queues.push(source.strip_prefix("subraumctl.recv.").unwrap().to_string());
                let destination = source.replace("recv", "send");
                let mut replies = Vec::new();
                match value["cmd"].as_str().unwrap() {
                    "pizza-list" => {
                        let message = format!("Orders requested by {}", value["user"].as_str().unwrap());
                        pending.push((destination.clone(), json!({"user":destination,"msg":message,"data":null}).to_string()));
                        if pending.len() == 2 {
                            // Reverse response order to catch shared reply routing bugs.
                            pending.reverse();
                            replies.append(&mut pending);
                        }
                    }
                    "pizza-add" => {
                        assert_eq!(value["user"], "alice");
                        assert_eq!(value["args"], "Quattro \"formaggi\"");
                        replies.push((destination.clone(), json!({"user":destination,"msg":"Sorry, we have already ordered!","data":null}).to_string()));
                    }
                    "pizza-services" => replies.push((destination, "not JSON".into())),
                    "pizza-lock" => {} // simulate an applied command without a reply
                    other => panic!("unexpected command: {other}"),
                }
                for (destination, body) in replies {
                    server_channel.basic_publish("msg", &destination, BasicPublishOptions::default(), body.as_bytes(), BasicProperties::default()).await.unwrap().await.unwrap();
                }
            }
            reply_queues
        });
        let options = Options { amqp_url: address, user: "alice".into(), debug: false, timeout: Duration::from_secs(3) };
        let alice = Request::new(Operation::List, "alice", &[]).unwrap();
        let bob = Request::new(Operation::List, "bob", &[]).unwrap();
        let (a, b) = tokio::join!(transport::execute(&options, &alice, false), transport::execute(&options, &bob, false));
        assert_eq!(a.unwrap()["message"], "Orders requested by alice");
        assert_eq!(b.unwrap()["message"], "Orders requested by bob");
        let request = Request::new(Operation::Add, "alice", &["Quattro \"formaggi\"".into()]).unwrap();
        let result = transport::execute(&options, &request, false).await.unwrap();
        // Business rejection must be preserved, not interpreted as accepted.
        assert_eq!(result["message"], "Sorry, we have already ordered!");
        let request = Request::new(Operation::Services, "alice", &[]).unwrap();
        assert!(transport::execute(&options, &request, false).await.unwrap_err().to_string().contains("invalid Pizzamat reply JSON"));
        let request = Request::new(Operation::Reset, "alice", &[]).unwrap();
        assert!(transport::execute(&options, &request, false).await.unwrap_err().to_string().contains("not routed"));
        let request = Request::new(Operation::Lock, "alice", &[]).unwrap();
        assert!(transport::execute(&options, &request, false).await.unwrap_err().to_string().contains("may have been applied"));
        for queue in fake_edi.await.unwrap() {
            let probe = connection.create_channel().await.unwrap();
            assert!(probe.queue_declare(&queue, QueueDeclareOptions { passive: true, ..Default::default() }, FieldTable::default()).await.is_err(), "reply queue leaked: {queue}");
        }
        connection.close(200, "done").await.unwrap();
    }).await.unwrap();
}
