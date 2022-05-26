use serde::{Serialize, Deserialize};
use futures::executor::block_on;

use reqwest::Client;
use reqwest_eventsource::RequestBuilderExt;
use futures::stream::StreamExt;

// Incoming Messages

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum Payload {
    Welcome,
    NewCamera,
    CameraDiscovery,
    CameraPing,
    CallInit,
    SDP {
        description: String
    },
    ICE {
        index: u32,
        candidate: String
    }
}

// Incoming Messages
#[derive(Clone, Deserialize)]
pub struct IncomingMessage {
    pub sender: String,
    pub payload: Payload
}

#[derive(Clone, Serialize)]
struct OutgoingMessage {
    pub recipient: Option<String>,
    pub payload: Payload
}

#[derive(Clone)]
pub struct Services {
    pub base_url: String,
    pub client: Client
}

impl Services {
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .cookie_store(true)
            .build()
            .unwrap();
        Services {
            base_url,
            client
        }
    }

    fn send_message<T: Serialize>(&self, message: &T) {
        let response = self.client
            .post(format!("{}/message", self.base_url))
            .json(&message)
            .send();

        let _response = match block_on(response) {
            Ok(response) => response,
            Err(error) => panic!("Problem sending the message:\n {:?}", error),
        };
    }

    pub fn send_camera_discovery(&self) {
        self.send_message(&OutgoingMessage {
            recipient: None,
            payload: Payload::CameraDiscovery
        });
    }

    pub fn send_call(&self, recipient: &String) {
        self.send_message(&OutgoingMessage {
            recipient: Some(recipient.to_string()),
            payload: Payload::CallInit
        });
    }

    pub fn send_sdp_offer(&self, recipient: &String, description: String) {
        self.send_message(&OutgoingMessage {
            recipient: Some(recipient.to_string()),
            payload: Payload::SDP {
                description
            }
        });
    }

    pub fn send_ice_candidate(&self, recipient: &String, index: u32, candidate: String) {
        self.send_message(&OutgoingMessage {
            recipient: Some(recipient.to_string()),
            payload: Payload::ICE {
                index,
                candidate
            }
        });
    }

    pub async fn start_sse<F1, F2, F3>(&self, on_call: F1, on_sdp_offer: F2, on_ice_candidate: F3) where
        F1: Fn(&String),
        F2: Fn(&IncomingMessage),
        F3: Fn(&IncomingMessage)
    {
        let mut stream = self.client
            .get(format!("{}/events", self.base_url))
            .eventsource()
            .unwrap();

        //self.send_sdp_offer(&"test".to_string(), "test".to_string());

        while let Some(event) = stream.next().await {
            match event {
                Ok(event) => {
                    let message: IncomingMessage = serde_json::from_str(&event.data).expect("Failed to parse received command");
                    match message.payload {
                        Payload::Welcome {..} => {
                            println!("Welcomed");
                            self.send_camera_discovery();
                        }
                        Payload::NewCamera => {
                            println!("New camera available");
                            self.send_call(&message.sender);
                            on_call(&message.sender);
                        }
                        Payload::CameraPing => {
                            println!("Camera ping");
                            self.send_call(&message.sender);
                            on_call(&message.sender);
                        }
                        Payload::SDP {..} => {
                            on_sdp_offer(&message);
                        },
                        Payload::ICE {..} => {
                            on_ice_candidate(&message);
                        }
                        _ => {
                            eprintln!("Error: Camera client no supposed to receive this payload type: {:?}", message.payload);
                        }
                    }
                },
                Err(error) => {
                    println!("=============");
                    println!("Error: {:?}", error);
                }
            }
        }
    }
}
