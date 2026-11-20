//! UDP transport for the crate's VRChat OSC face-tracking messages.

use std::io;
use std::net::{SocketAddr, UdpSocket};

use crate::osc::{encode_face_v1, FaceExpressions};
use crate::EyeSample;

/// Connected UDP sender for OSC face-tracking packets.
///
/// The caller owns tracker acquisition and scheduling; this type only binds
/// a local endpoint and sends one encoded packet per call.
pub struct OscSender {
    socket: UdpSocket,
}

impl OscSender {
    /// Bind `local` and connect the UDP socket to `remote`.
    pub fn bind(local: SocketAddr, remote: SocketAddr) -> io::Result<Self> {
        let socket = UdpSocket::bind(local)?;
        socket.connect(remote)?;
        Ok(Self { socket })
    }

    /// Encode and send one VRChat `/tracking/face/v1` OSC packet.
    pub fn send_face_v1(
        &self,
        left_gaze: EyeSample,
        right_gaze: EyeSample,
        expressions: FaceExpressions,
    ) -> io::Result<usize> {
        let packet = encode_face_v1(left_gaze, right_gaze, expressions);
        self.socket.send(&packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    #[test]
    fn reports_bind_and_connect_failures() {
        let occupied = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let error = OscSender::bind(
            occupied.local_addr().unwrap(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 9000),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);

        let result = OscSender::bind(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), 9000),
        );
        assert!(result.is_err());
    }

    #[test]
    fn sends_the_encoded_face_packet_to_the_connected_peer() {
        let receiver = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let sender = OscSender::bind(
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            receiver.local_addr().unwrap(),
        )
        .unwrap();
        let left = EyeSample {
            yaw_deg: 10.0,
            pitch_deg: -5.0,
        };
        let right = EyeSample {
            yaw_deg: 20.0,
            pitch_deg: 5.0,
        };
        let expressions = FaceExpressions::default();
        let expected = encode_face_v1(left, right, expressions);

        assert_eq!(
            sender.send_face_v1(left, right, expressions).unwrap(),
            expected.len()
        );
        let mut packet = vec![0; 512];
        let size = receiver.recv(&mut packet).unwrap();
        assert_eq!(&packet[..size], expected);
    }
}
