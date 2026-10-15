use fluidity_mgmt::{Client, Error, Event, PasswordRequest, ProxyAction, Response, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

struct Server {
    lines: tokio::io::Lines<BufReader<tokio::io::ReadHalf<DuplexStream>>>,
    write: tokio::io::WriteHalf<DuplexStream>,
}

impl Server {
    async fn send(&mut self, text: &str) {
        self.write.write_all(text.as_bytes()).await.unwrap();
    }

    async fn expect(&mut self, line: &str) {
        assert_eq!(self.lines.next_line().await.unwrap().unwrap(), line);
    }
}

fn pair() -> (Client, tokio::sync::mpsc::UnboundedReceiver<Event>, Server) {
    let (a, b) = tokio::io::duplex(4096);
    let (client, events) = Client::new(a);
    let (read, write) = tokio::io::split(b);
    (
        client,
        events,
        Server {
            lines: BufReader::new(read).lines(),
            write,
        },
    )
}

#[tokio::test]
async fn single_and_multiline_responses_interleaved_with_events() {
    let (client, mut events, mut server) = pair();

    let task = tokio::spawn({
        let client = client.clone();
        async move {
            let a = client.hold_release().await.unwrap();
            let b = client.command("status").await.unwrap();
            let c = client.command("bogus").await;
            (a, b, c)
        }
    });

    server.send(">INFO:OpenVPN Management Interface Version 5\r\n").await;
    server.expect("hold release").await;
    server
        .send(">HOLD:Waiting for hold release:0\nSUCCESS: hold release succeeded\n")
        .await;
    server.expect("status").await;
    server
        .send("OpenVPN CLIENT LIST\n>BYTECOUNT:1,2\nUpdated,now\nEND\n")
        .await;
    server.expect("bogus").await;
    server.send("ERROR: unknown command\n").await;

    let (a, b, c) = task.await.unwrap();
    assert_eq!(a, Response::Success("hold release succeeded".into()));
    assert_eq!(b.lines(), ["OpenVPN CLIENT LIST", "Updated,now"]);
    assert!(matches!(c, Err(Error::Rejected(m)) if m == "unknown command"));

    assert!(matches!(events.recv().await, Some(Event::Info(_))));
    assert!(matches!(events.recv().await, Some(Event::Hold(_))));
    assert_eq!(
        events.recv().await,
        Some(Event::ByteCount {
            bytes_in: 1,
            bytes_out: 2
        })
    );
}

#[tokio::test]
async fn auth_flow_quotes_arguments() {
    let (client, mut events, mut server) = pair();
    server.send(">PASSWORD:Need 'Auth' username/password\n").await;
    let Some(Event::Password(PasswordRequest::Need { realm, .. })) = events.recv().await else {
        panic!()
    };

    let task = tokio::spawn({
        let client = client.clone();
        async move { client.credentials(&realm, "al\"ice", r"pa\ss word").await }
    });
    server.expect(r#"username "Auth" "al\"ice""#).await;
    server
        .send("SUCCESS: 'Auth' username entered, but not yet verified\n")
        .await;
    server.expect(r#"password "Auth" "pa\\ss word""#).await;
    server
        .send("SUCCESS: 'Auth' password entered, but not yet verified\n")
        .await;
    task.await.unwrap().unwrap();

    server
        .send(">STATE:1,CONNECTED,SUCCESS,10.8.0.2,1.2.3.4,1194,,,\n")
        .await;
    let Some(Event::State(s)) = events.recv().await else {
        panic!()
    };
    assert_eq!(s.state, State::Connected);
}

#[tokio::test]
async fn proxy_pkcs11_and_updown() {
    let (client, mut events, mut server) = pair();
    let task = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .proxy(ProxyAction::Http {
                    host: "p".into(),
                    port: 8080,
                    nct: true,
                })
                .await
                .unwrap();
            client.pkcs11_ids().await.unwrap()
        }
    });
    server.expect(r#"proxy HTTP "p" 8080 nct"#).await;
    server.send("SUCCESS: proxy command succeeded\n").await;
    server.expect("pkcs11-id-count").await;
    server.send(">PKCS11ID-COUNT:1\n").await;
    server.expect("pkcs11-id-get 0").await;
    server.send(">PKCS11ID-ENTRY:'0', ID:'abc', BLOB:'xyz'\n").await;
    assert_eq!(task.await.unwrap(), ["'0', ID:'abc', BLOB:'xyz'"]);

    server
        .send(">UPDOWN:UP\n>UPDOWN:ENV,dev=utun5\n>UPDOWN:ENV,ifconfig_local=10.0.0.2\n>UPDOWN:ENV,END\n")
        .await;
    assert_eq!(
        events.recv().await,
        Some(Event::UpDown {
            direction: "UP".into(),
            env: vec![
                ("dev".into(), "utun5".into()),
                ("ifconfig_local".into(), "10.0.0.2".into())
            ]
        })
    );
}

#[tokio::test]
async fn pending_commands_fail_when_closed() {
    let (client, _events, server) = pair();
    let task = tokio::spawn(async move { client.hold_release().await });
    tokio::task::yield_now().await;
    drop(server);
    assert!(matches!(task.await.unwrap(), Err(Error::Closed) | Err(Error::Io(_))));
}
