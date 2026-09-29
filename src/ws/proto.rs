//! Protobuf frames used by the Feishu and Lark long-connection protocol.

#[derive(Clone, PartialEq, prost::Message)]
pub struct Header {
    #[prost(string, required, tag = "1")]
    pub key: String,
    #[prost(string, required, tag = "2")]
    pub value: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Frame {
    #[prost(uint64, required, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, required, tag = "2")]
    pub log_id: u64,
    #[prost(int32, required, tag = "3")]
    pub service: i32,
    #[prost(int32, required, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<Header>,
    #[prost(string, optional, tag = "6")]
    pub payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    pub payload_type: Option<String>,
    #[prost(bytes = "vec", optional, tag = "8")]
    pub payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    pub log_id_new: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn frame_round_trips_binary_wire_format() {
        let frame = Frame {
            seq_id: 7,
            log_id: 9,
            service: 11,
            method: 1,
            headers: vec![Header {
                key: "type".into(),
                value: "event".into(),
            }],
            payload_encoding: None,
            payload_type: None,
            payload: Some(br#"{"header":{}}"#.to_vec()),
            log_id_new: None,
        };
        let bytes = frame.encode_to_vec();
        let decoded = Frame::decode(bytes.as_ref()).unwrap();

        assert_eq!(frame, decoded);
    }
}
