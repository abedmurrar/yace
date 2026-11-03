//! JSON-RPC 2.0 framing over stdio (`Content-Length` headers).

use std::io::{self, BufRead, Write};

use serde_json::Value;

/// Read one message body. `Ok(None)` at end of stream.
pub fn read_message(r: &mut impl BufRead, buf: &mut Vec<u8>) -> io::Result<Option<Value>> {
    let mut len: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let l = line.trim_end();
        if l.is_empty() {
            if len.is_some() {
                break;
            }
            continue;
        }
        if let Some((k, v)) = l.split_once(':')
            && k.eq_ignore_ascii_case("content-length")
        {
            len = v.trim().parse().ok();
        }
    }
    let n = len.unwrap_or(0);
    buf.resize(n, 0);
    r.read_exact(buf)?;
    serde_json::from_slice(buf).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn write_message(w: &mut impl Write, msg: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()
}

pub fn request(id: u64, method: &str, params: Value) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

pub fn notification(method: &str, params: Value) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

pub fn response(id: Value, result: Value) -> Value {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_multiple_messages() {
        let mut out = Vec::new();
        write_message(&mut out, &request(1, "a", serde_json::json!({"x": "é"}))).unwrap();
        write_message(&mut out, &notification("b", Value::Null)).unwrap();
        let mut r = io::BufReader::new(&out[..]);
        let mut buf = Vec::new();
        let m1 = read_message(&mut r, &mut buf).unwrap().unwrap();
        assert_eq!(m1["params"]["x"], "é");
        let m2 = read_message(&mut r, &mut buf).unwrap().unwrap();
        assert_eq!(m2["method"], "b");
        assert!(read_message(&mut r, &mut buf).unwrap().is_none());
    }
}
