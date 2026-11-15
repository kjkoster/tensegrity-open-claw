//! OSC's framing: messages, bundles nested to any depth, padded strings. Only the types the
//! fixture files may name, `i f s T F`; a packet with any other is refused whole, rather than
//! recorded in part.

use std::fmt;

/// One value, as OSC carried it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i32),
    Float(f32),
    Text(String),
    Bool(bool),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Text(s) => f.write_str(s),
            Value::Bool(b) => write!(f, "{b}"),
        }
    }
}

/// Every message in `packet`, in order, each its address and its values. `Err` says why the
/// packet is not one this decoder takes.
pub fn decode(packet: &[u8]) -> Result<Vec<(String, Vec<Value>)>, String> {
    let mut found = Vec::new();
    element(packet, &mut found)?;
    Ok(found)
}

fn element(packet: &[u8], found: &mut Vec<(String, Vec<Value>)>) -> Result<(), String> {
    if let Some(rest) = packet.strip_prefix(b"#bundle\0") {
        // The time tag: what was sent is recorded when it arrives.
        let mut elements = rest.get(8..).ok_or("a bundle with no time tag")?;
        while !elements.is_empty() {
            let size = elements.get(..4).ok_or("a bundle element with no size")?;
            let size = u32::from_be_bytes(size.try_into().unwrap()) as usize;
            let inner = elements
                .get(4..4 + size)
                .ok_or("a bundle element past the packet")?;
            element(inner, found)?;
            elements = &elements[4 + size..];
        }
        return Ok(());
    }
    let mut at = 0;
    let address = string(packet, &mut at)?;
    if !address.starts_with('/') {
        return Err(format!("the address {address:?}"));
    }
    let tags = string(packet, &mut at)?;
    let tags = tags.strip_prefix(',').ok_or("no type tags")?;
    let mut values = Vec::with_capacity(tags.len());
    for tag in tags.chars() {
        values.push(match tag {
            'i' => Value::Int(i32::from_be_bytes(word(packet, &mut at)?)),
            'f' => Value::Float(f32::from_be_bytes(word(packet, &mut at)?)),
            's' => Value::Text(string(packet, &mut at)?.to_owned()),
            'T' => Value::Bool(true),
            'F' => Value::Bool(false),
            other => return Err(format!("{address}: the type {other:?}")),
        });
    }
    found.push((address.to_owned(), values));
    Ok(())
}

/// The OSC string at `at`, moving `at` past it and its padding to four bytes.
fn string<'a>(packet: &'a [u8], at: &mut usize) -> Result<&'a str, String> {
    let rest = packet.get(*at..).ok_or("a string past the packet")?;
    let len = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or("an unterminated string")?;
    *at += (len / 4 + 1) * 4;
    std::str::from_utf8(&rest[..len]).map_err(|e| format!("a string that is not UTF-8: {e}"))
}

fn word(packet: &[u8], at: &mut usize) -> Result<[u8; 4], String> {
    let bytes = packet.get(*at..*at + 4).ok_or("a value past the packet")?;
    *at += 4;
    Ok(bytes.try_into().unwrap())
}
