// ---------- DDC/CI over i2c-dev ----------

use crate::ddc::{self, edid_name};
use crate::log::{Logged, log};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::thread::sleep;
use std::time::Duration;

const I2C_SLAVE: u64 = 0x0703;

fn open_i2c(bus: &str, address: u16) -> std::io::Result<File> {
    let file = OpenOptions::new().read(true).write(true).open(bus)?;
    if unsafe { libc::ioctl(file.as_raw_fd(), I2C_SLAVE as _, address as libc::c_ulong) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

pub(crate) struct Monitor {
    pub(crate) name: String,
    file: File,
}

pub(crate) fn monitors() -> Vec<Monitor> {
    let mut buses: Vec<String> = fs::read_dir("/dev")
        .map(|entries| entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).filter(|name| name.starts_with("i2c-")).collect())
        .unwrap_or_default();
    buses.sort_by_key(|name| name[4..].parse::<u32>().unwrap_or(0));
    let (mut found, mut seen) = (vec![], vec![]);
    for bus in buses {
        // never poke SMBus adapters (RAM SPD lives at 0x50 there)
        if fs::read_to_string(format!("/sys/bus/i2c/devices/{bus}/name")).is_ok_and(|name| name.to_lowercase().contains("smbus")) {
            continue;
        }
        let device = format!("/dev/{bus}");
        let edid = open_i2c(&device, 0x50).map_err(|error| format!("{device}: {error} (módulo i2c-dev carregado? usuário no grupo i2c?)")).logged("abrir i2c").ok().and_then(|mut file| {
            file.write_all(&[0]).ok()?;
            let mut edid = [0u8; 128];
            file.read_exact(&mut edid).ok()?;
            Some(edid)
        });
        let Some(edid) = edid else {
            log(format!("{device}: sem EDID (não é monitor)"));
            continue;
        };
        // same monitor is often reachable on several buses
        let (Some(name), false) = (edid_name(&edid), seen.contains(&edid)) else { continue };
        seen.push(edid);
        match open_i2c(&device, 0x37) {
            Ok(file) => found.push(Monitor { name, file }),
            Err(error) => log(format!("ERRO {device} ({name}) DDC 0x37: {error}")),
        }
    }
    found
}

impl Monitor {
    fn send(&self, payload: &[u8]) -> Result<(), String> {
        (&self.file).write_all(&ddc::frame(payload)).map_err(|error| format!("{} i2c write: {error}", self.name))
    }

    fn receive(&self, length: usize) -> Result<Vec<u8>, String> {
        let mut buffer = vec![0; length];
        (&self.file).read_exact(&mut buffer).map(|_| buffer).map_err(|error| format!("{} i2c read: {error}", self.name))
    }

    /// Standard VCP (1-byte op) or vendor opcode (AOC F8 E1 xx): same framing on i2c-dev.
    pub(crate) fn raw_get(&self, opcode: &[u8]) -> Result<(u16, u16), String> {
        let mut last_error = String::new();
        for _ in 0..3 {
            self.send(&[&[0x01], opcode].concat())?;
            sleep(Duration::from_millis(50));
            match self.receive(11) {
                Ok(reply) => match ddc::reply(&reply, opcode) {
                    Some(parsed) => return Ok(parsed),
                    None => last_error = format!("resposta inválida {reply:02X?}"),
                },
                Err(error) => last_error = error,
            }
            sleep(Duration::from_millis(50));
        }
        Err(format!("{opcode:02X?}: {last_error}"))
    }

    pub(crate) fn raw_set(&self, opcode: &[u8], value: u16) -> Result<(), String> {
        self.send(&[&[0x03], opcode, &value.to_be_bytes()].concat())?;
        sleep(Duration::from_millis(50));
        Ok(())
    }

    pub(crate) fn get(&self, code: u8) -> Result<(u16, u16), String> {
        self.raw_get(&[code])
    }

    pub(crate) fn set(&self, code: u8, value: u16) -> Result<(), String> {
        self.raw_set(&[code], value)
    }

    /// DDC/CI "Save Current Settings" (0x0C): commit to the monitor's NVRAM.
    pub(crate) fn save(&self) -> Result<(), String> {
        self.send(&[0x0C])?;
        sleep(Duration::from_millis(200));
        Ok(())
    }

    pub(crate) fn capabilities(&self) -> Result<String, String> {
        let (mut text, mut offset) = (vec![], 0u16);
        loop {
            let mut chunk = None;
            for _ in 0..3 {
                self.send(&[0xF3, (offset >> 8) as u8, offset as u8])?;
                sleep(Duration::from_millis(50));
                // 6E len E3 offH offL data... chk
                chunk = self.receive(38).logged("caps").ok().filter(|reply| reply[2] == 0xE3 && u16::from_be_bytes([reply[3], reply[4]]) == offset);
                if chunk.is_some() {
                    break;
                }
            }
            let reply = chunk.ok_or("monitor não respondeu ao DDC/CI")?;
            let length = ((reply[1] & 0x7F) as usize).saturating_sub(3).min(32);
            if length == 0 {
                break;
            }
            text.extend_from_slice(&reply[5..5 + length]);
            offset += length as u16;
        }
        Ok(String::from_utf8_lossy(&text).trim_end_matches('\0').to_string())
    }
}
