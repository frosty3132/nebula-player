//! Мини-демультиплексор MPEG-TS: достаёт из HLS-сегмента аудиопоток
//! (ADTS AAC или MPEG audio) — symphonia не умеет читать TS напрямую.
//! Все обращения к байтам проверяются, битые пакеты просто пропускаются.

const PACKET: usize = 188;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EsKind {
    Adts,
    Mpeg,
}

fn section(payload: &[u8], pusi: bool) -> Option<&[u8]> {
    if !pusi {
        return None;
    }
    let ptr = *payload.first()? as usize;
    payload.get(1 + ptr..)
}

pub fn demux_audio(data: &[u8]) -> Option<(EsKind, Vec<u8>)> {
    let mut pmt_pid: Option<u16> = None;
    let mut audio: Option<(u16, EsKind)> = None;
    let mut out = Vec::with_capacity(data.len() * 9 / 10);
    let mut pes_open = false;

    let mut i = data.iter().position(|&b| b == 0x47)?;
    while i + PACKET <= data.len() {
        let p = &data[i..i + PACKET];
        if p[0] != 0x47 {
            // потеряли синхронизацию — ищем следующий sync-байт
            match data[i + 1..].iter().position(|&b| b == 0x47) {
                Some(off) => {
                    i += 1 + off;
                    continue;
                }
                None => break,
            }
        }
        i += PACKET;

        let pusi = p[1] & 0x40 != 0;
        let pid = (((p[1] & 0x1f) as u16) << 8) | p[2] as u16;
        let afc = (p[3] >> 4) & 0x3;
        let mut off = 4usize;
        if afc == 2 || afc == 3 {
            off += 1 + p[4] as usize;
        }
        if afc == 2 || afc == 0 || off >= PACKET {
            continue;
        }
        let payload = &p[off..];

        if pid == 0 {
            if let Some(sec) = section(payload, pusi) {
                if sec.first() != Some(&0x00) || sec.len() < 8 {
                    continue;
                }
                let len = (((sec[1] & 0x0f) as usize) << 8) | sec[2] as usize;
                let end = (3 + len).saturating_sub(4).min(sec.len());
                if end > 8 {
                    for e in sec[8..end].chunks_exact(4) {
                        let num = ((e[0] as u16) << 8) | e[1] as u16;
                        if num != 0 {
                            pmt_pid = Some((((e[2] & 0x1f) as u16) << 8) | e[3] as u16);
                            break;
                        }
                    }
                }
            }
            continue;
        }

        if Some(pid) == pmt_pid {
            if audio.is_none() {
                if let Some(sec) = section(payload, pusi) {
                    if sec.first() != Some(&0x02) || sec.len() < 12 {
                        continue;
                    }
                    let len = (((sec[1] & 0x0f) as usize) << 8) | sec[2] as usize;
                    let end = (3 + len).saturating_sub(4).min(sec.len());
                    let pil = (((sec[10] & 0x0f) as usize) << 8) | sec[11] as usize;
                    let mut j = 12 + pil;
                    while j + 5 <= end {
                        let stype = sec[j];
                        let epid = (((sec[j + 1] & 0x1f) as u16) << 8) | sec[j + 2] as u16;
                        let info_len = (((sec[j + 3] & 0x0f) as usize) << 8) | sec[j + 4] as usize;
                        let kind = match stype {
                            0x0f => Some(EsKind::Adts),
                            0x03 | 0x04 => Some(EsKind::Mpeg),
                            _ => None,
                        };
                        if let Some(k) = kind {
                            audio = Some((epid, k));
                            break;
                        }
                        j += 5 + info_len;
                    }
                }
            }
            continue;
        }

        if let Some((apid, _)) = audio {
            if pid != apid {
                continue;
            }
            if pusi {
                pes_open = false;
                if payload.len() >= 9 && payload[0] == 0 && payload[1] == 0 && payload[2] == 1 {
                    let start = 9 + payload[8] as usize;
                    if let Some(body) = payload.get(start..) {
                        out.extend_from_slice(body);
                        pes_open = true;
                    }
                }
            } else if pes_open {
                out.extend_from_slice(payload);
            }
        }
    }

    audio.map(|(_, k)| (k, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(pid: u16, pusi: bool, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0xffu8; PACKET];
        p[0] = 0x47;
        p[1] = ((pid >> 8) as u8 & 0x1f) | if pusi { 0x40 } else { 0 };
        p[2] = pid as u8;
        // adaptation field со стаффингом, чтобы полезная нагрузка встала ровно в конец
        let room = PACKET - 4;
        if payload.len() < room {
            p[3] = 0x30;
            let af_len = room - 1 - payload.len();
            p[4] = af_len as u8;
            if af_len > 0 {
                p[5] = 0x00;
            }
            p[PACKET - payload.len()..].copy_from_slice(payload);
        } else {
            p[3] = 0x10;
            p[4..].copy_from_slice(&payload[..room]);
        }
        p
    }

    #[test]
    fn extracts_adts_from_pes() {
        let pat = [0x00, 0x00, 0xb0, 0x0d, 0x00, 0x01, 0xc1, 0x00, 0x00, 0x00, 0x01, 0xf0, 0x00, 0, 0, 0, 0];
        let pmt = [0x00, 0x02, 0xb0, 0x12, 0x00, 0x01, 0xc1, 0x00, 0x00, 0xe1, 0x00, 0xf0, 0x00, 0x0f, 0xe1, 0x01, 0xf0, 0x00, 0, 0, 0, 0];
        let es = [0xff, 0xf1, 0x50, 0x80, 0x01, 0x7f, 0xfc, 0xde, 0xad];
        let mut pes = vec![0x00, 0x00, 0x01, 0xc0, 0x00, 0x00, 0x80, 0x00, 0x00];
        pes.extend_from_slice(&es);
        let mut ts = packet(0, true, &pat);
        ts.extend(packet(0x1000, true, &pmt));
        ts.extend(packet(0x101, true, &pes));
        let (kind, out) = demux_audio(&ts).expect("audio stream");
        assert_eq!(kind, EsKind::Adts);
        assert_eq!(out, es);
    }
}
