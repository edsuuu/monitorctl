// ---------- DDC/CI framing (raw I2C paths: Linux i2c-dev, NVAPI) ----------

/// [0x51, 0x80|len, payload.., chk]; chk = XOR including the 0x6E destination address.
pub(crate) fn frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x51, 0x80 | payload.len() as u8];
    frame.extend_from_slice(payload);
    frame.push(frame.iter().fold(0x6E, |checksum, byte| checksum ^ byte));
    frame
}

/// Parse a VCP reply `6E 88 02 rc op.. maxH maxL curH curL chk` -> (cur, max).
/// AOC `F8 E1 sub` replies echo `F8 sub` in bytes 4-5; checking it rejects stale buffers (NVAPI returns the
/// previous reply when the monitor ignores an opcode it doesn't support).
pub(crate) fn reply(bytes: &[u8], opcode: &[u8]) -> Option<(u16, u16)> {
    if bytes.len() < 11 || bytes[0] != 0x6E || bytes[1] & 0x80 == 0 || bytes[2] != 0x02 || (opcode.len() == 1 && (bytes[3] != 0 || bytes[4] != opcode[0])) || (opcode.len() == 3 && (bytes[4] != opcode[0] || bytes[5] != opcode[2])) {
        return None;
    }
    Some((u16::from_be_bytes([bytes[8], bytes[9]]), u16::from_be_bytes([bytes[6], bytes[7]])))
}

/// Parse the `vcp(...)` section of a DDC/CI capabilities string into (code, allowed values).
pub(crate) fn parse_vcp(capabilities: &str) -> Vec<(u8, Vec<u16>)> {
    let Some(start) = capabilities.find("vcp(") else { return vec![] };
    let (mut codes, mut depth, mut token) = (Vec::<(u8, Vec<u16>)>::new(), 0, String::new());
    let flush = |token: &mut String, depth: i32, codes: &mut Vec<(u8, Vec<u16>)>| {
        match (depth, codes.last_mut()) {
            // codes: some monitors omit spaces ("100204"), so split every 2 hex digits
            (0, _) => codes.extend(token.as_bytes().chunks(2).filter_map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()).map(|code| (code, vec![]))),
            // values: Dell lists 16-bit ones ("ED(0400 0800)", "F5(100 500)")
            (_, Some(last)) if !token.is_empty() => last.1.extend(u16::from_str_radix(token, 16).ok()),
            _ => {}
        }
        token.clear();
    };
    for character in capabilities[start + 4..].chars() {
        match character {
            '(' => {
                flush(&mut token, depth, &mut codes);
                depth += 1;
            }
            ')' => {
                flush(&mut token, depth, &mut codes);
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            hex_digit if hex_digit.is_ascii_hexdigit() => token.push(hex_digit),
            _ => flush(&mut token, depth, &mut codes),
        }
    }
    codes
}

/// Monitor name from a 128-byte EDID block (descriptor 0xFC), else PNP id + product code.
#[cfg(any(target_os = "linux", test))] // Windows gets names from the CCD API
pub(crate) fn edid_name(edid: &[u8]) -> Option<String> {
    if edid.len() < 128 || edid[..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] {
        return None;
    }
    for descriptor in edid[54..126].chunks(18) {
        if descriptor[..3] == [0, 0, 0] && descriptor[3] == 0xFC {
            return Some(String::from_utf8_lossy(&descriptor[5..]).split('\n').next().unwrap_or("").trim().to_string());
        }
    }
    let manufacturer = u16::from_be_bytes([edid[8], edid[9]]);
    let letter = |shift: u16| (b'@' + (manufacturer >> shift & 31) as u8) as char;
    Some(format!("{}{}{}{:02X}{:02X}", letter(10), letter(5), letter(0), edid[11], edid[10]))
}

/// "10" -> [0x10]; "f8e151" -> [F8, E1, 51]
pub(crate) fn opcode(text: &str) -> Result<Vec<u8>, String> {
    let text = text.trim_start_matches("0x");
    let bytes: Option<Vec<u8>> = (0..text.len()).step_by(2).map(|index| text.get(index..index + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok())).collect();
    bytes.filter(|bytes| !bytes.is_empty()).ok_or(format!("código inválido: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_parser() {
        // trimmed from the real AW2525HM string
        let capabilities = "(prot(monitor)cmds(01 02 03 0C E3 F3)vcp(02 10 14(01 05 08 0B 0C) 60( 0F 11 12) E2(00 04 0E)E3 EC(FF02) ED(0400 0800 03FF) F5(100 500) E0FF)mccs_ver(2.1))";
        let codes = parse_vcp(capabilities);
        assert_eq!(codes.iter().map(|code| code.0).collect::<Vec<_>>(), [0x02, 0x10, 0x14, 0x60, 0xE2, 0xE3, 0xEC, 0xED, 0xF5, 0xE0, 0xFF]);
        assert_eq!(codes[2].1, [1, 5, 8, 0x0B, 0x0C]);
        assert_eq!(codes[3].1, [0x0F, 0x11, 0x12]);
        assert!(codes[5].1.is_empty());
        assert_eq!(codes[6].1, [0xFF02]);
        assert_eq!(codes[7].1, [0x400, 0x800, 0x3FF]);
        assert_eq!(codes[8].1, [0x100, 0x500]);
    }

    #[test]
    fn edid() {
        let mut edid = [0u8; 128];
        edid[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
        edid[8..10].copy_from_slice(&[0x10, 0xAC]); // DEL
        edid[10..12].copy_from_slice(&[0x8F, 0x43]);
        assert_eq!(edid_name(&edid).unwrap(), "DEL438F");
        edid[72..77].copy_from_slice(&[0, 0, 0, 0xFC, 0]);
        edid[77..86].copy_from_slice(b"AW2525HM\n");
        assert_eq!(edid_name(&edid).unwrap(), "AW2525HM");
    }

    #[test]
    fn framing() {
        // MCCS example: get brightness = 51 82 01 10 AC
        assert_eq!(frame(&[0x01, 0x10]), [0x51, 0x82, 0x01, 0x10, 0xAC]);
        let mut bytes = [0x6E, 0x88, 0x02, 0x00, 0x10, 0x00, 0x00, 0x64, 0x00, 0x32, 0];
        assert_eq!(reply(&bytes, &[0x10]), Some((50, 100)));
        bytes[3] = 1; // rc error
        assert_eq!(reply(&bytes, &[0x10]), None);
        // real 25G3ZM replies: overdrive, and a stale overdrive buffer returned for unsupported 0x71
        let overdrive = [0x6E, 0x88, 0x02, 0x00, 0xF8, 0x51, 0x00, 0x03, 0x00, 0x03, 0x1D];
        assert_eq!(reply(&overdrive, &[0xF8, 0xE1, 0x51]), Some((3, 3)));
        assert_eq!(reply(&overdrive, &[0xF8, 0xE1, 0x71]), None);
        assert_eq!(opcode("f8e151").unwrap(), [0xF8, 0xE1, 0x51]);
        assert_eq!(opcode("0x10").unwrap(), [0x10]);
        assert!(crate::log::now().len() == 24);
    }
}
