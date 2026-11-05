//! Authenticated, boot-fenced C0 requests; native work never blocks the config actor.
use super::experience::{self, PeerRequest};
use crate::{
    api::DeviceKey,
    node::{Node, lock},
};
use oink_protocol::v1::{
    Message, dto,
    schema::{FixedBytes, WireDto, WireValue},
};
use std::{sync::mpsc, time::Duration};

pub(super) fn request(node: &Node, peer: DeviceKey, request: PeerRequest) -> Result<serde_json::Value, String> {
    if peer.0 == node.id() {
        return experience::receive(node, node.id().into(), request);
    }
    let config = node.link_config().map_err(|e| e.message().to_string())?;
    if !config.peer_supports(peer.0, experience::CAP) {
        return Err("This computer needs an Oink version supporting safe physical monitor switching".into());
    }
    let target = config
        .peer_boot(peer.0)
        .ok_or("The participating computer is offline")?;
    let id = crate::api::request_context::new_client();
    let body = serde_json::to_string(&request).map_err(|e| e.to_string())?;
    if body.len() > 32768 {
        return Err("Physical monitor request is too large".into());
    }
    let wire = dto::PhysicalMonitorRequest::from_raw(
        [
            (1, FixedBytes(id).write()),
            (2, oink_network::wire_identity::boot(target).write()),
            (3, oink_protocol::v1::RawValue::Text(body)),
        ]
        .into(),
    )
    .map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::channel();
    lock(&node.inner.physical.pending).insert((peer, id), (target, tx));
    let outcome = (|| {
        config
            .send_device_control(peer.0, Message::PhysicalMonitorRequest(wire))
            .map_err(|e| e.message().to_string())?;
        rx.recv_timeout(Duration::from_secs(35))
            .map_err(|_| "The computer did not answer; its local recovery guard remains armed".to_string())?
    })();
    lock(&node.inner.physical.pending).remove(&(peer, id));
    outcome
}
impl Node {
    pub(crate) fn receive_physical_monitor(&self, peer: DeviceKey, message: Message) {
        match message {
            Message::PhysicalMonitorResult(reply) => {
                let (Ok(id), Ok(target), Ok(body)) = (reply.request_id(), reply.target(), reply.body()) else {
                    return;
                };
                let Ok(boot) = oink_network::wire_identity::read_boot(&target) else {
                    return;
                };
                if let Some((expected, tx)) = lock(&self.inner.physical.pending).get(&(peer, id.0))
                    && *expected == boot
                {
                    let result: Result<serde_json::Value, String> = serde_json::from_str(&body.0)
                        .unwrap_or_else(|_| Err("Invalid physical monitor response".into()));
                    let _ = tx.send(result);
                }
            }
            Message::PhysicalMonitorRequest(request) => {
                let (Ok(target), Ok(id), Ok(body)) = (request.target(), request.request_id(), request.body()) else {
                    return;
                };
                if oink_network::wire_identity::read_boot(&target) != Ok(self.boot()) {
                    return;
                }
                let node = self.clone();
                self.inner.rt.spawn_blocking(move || {
                    let admitted = node.link_config().ok().is_some_and(|c| {
                        c.applied().members.contains(&peer.0) && c.peer_supports(peer.0, experience::CAP)
                    });
                    if !admitted {
                        return;
                    }
                    let result = serde_json::from_str::<PeerRequest>(&body.0)
                        .map_err(|_| "Invalid physical monitor request".into())
                        .and_then(|r| experience::receive(&node, peer, r));
                    let Ok(body) = serde_json::to_string(&result) else {
                        return;
                    };
                    if body.len() > 32768 {
                        return;
                    }
                    if let Ok(reply) = dto::PhysicalMonitorResult::from_raw(
                        [
                            (1, id.write()),
                            (2, target.write()),
                            (3, oink_protocol::v1::RawValue::Text(body)),
                        ]
                        .into(),
                    ) && let Ok(config) = node.link_config()
                    {
                        let _ = config.send_device_control(peer.0, Message::PhysicalMonitorResult(reply));
                    }
                });
            }
            _ => {}
        }
    }
}
