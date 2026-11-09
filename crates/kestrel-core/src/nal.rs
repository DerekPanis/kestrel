//! Minimal Annex-B inspection: split NAL units, detect keyframes and build
//! WebCodecs codec strings. libchiaki hands us one access unit per callback.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    H265,
}

/// Iterates NAL unit payloads (without start codes) in an Annex-B buffer.
pub fn nal_units(buf: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (n, &s) in starts.iter().enumerate() {
        let mut end = if n + 1 < starts.len() {
            starts[n + 1] - 3
        } else {
            buf.len()
        };
        // A 4-byte start code leaves one extra zero before the next unit.
        while end > s && buf[end - 1] == 0 && n + 1 < starts.len() {
            end -= 1;
        }
        units.push(&buf[s..end]);
    }
    units.into_iter()
}

fn nal_type(codec: Codec, nal: &[u8]) -> Option<u8> {
    let h = *nal.first()?;
    Some(match codec {
        Codec::H264 => h & 0x1f,
        Codec::H265 => (h >> 1) & 0x3f,
    })
}

/// True when the access unit can start decoding (IDR / IRAP picture).
pub fn is_keyframe(codec: Codec, au: &[u8]) -> bool {
    nal_units(au).any(|n| match (codec, nal_type(codec, n)) {
        (Codec::H264, Some(5)) => true,
        (Codec::H265, Some(t)) => (16..=21).contains(&t),
        _ => false,
    })
}

/// Finds the sequence parameter set in an access unit.
pub fn find_sps(codec: Codec, au: &[u8]) -> Option<&[u8]> {
    nal_units(au).find(|n| {
        matches!(
            (codec, nal_type(codec, n)),
            (Codec::H264, Some(7)) | (Codec::H265, Some(33))
        )
    })
}

/// `avc1.PPCCLL` from an H.264 SPS (profile_idc, constraint flags, level_idc).
pub fn h264_codec_string(sps: &[u8]) -> Option<String> {
    if sps.len() < 4 || sps[0] & 0x1f != 7 {
        return None;
    }
    Some(format!("avc1.{:02x}{:02x}{:02x}", sps[1], sps[2], sps[3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const H264_IDR_AU: &[u8] = &[
        0, 0, 0, 1, 0x67, 0x64, 0x00, 0x2a, 0xac, // SPS: High, level 4.2
        0, 0, 0, 1, 0x68, 0xee, 0x3c, 0x80, // PPS
        0, 0, 1, 0x65, 0x88, 0x84, 0x00, // IDR slice (3-byte start code)
    ];
    const H264_P_AU: &[u8] = &[0, 0, 0, 1, 0x41, 0x9a, 0x02];

    #[test]
    fn splits_units() {
        let units: Vec<_> = nal_units(H264_IDR_AU).collect();
        assert_eq!(units.len(), 3);
        assert_eq!(units[0][0], 0x67);
        assert_eq!(units[1], &[0x68, 0xee, 0x3c, 0x80]);
        assert_eq!(units[2][0], 0x65);
    }

    #[test]
    fn h264_keyframes() {
        assert!(is_keyframe(Codec::H264, H264_IDR_AU));
        assert!(!is_keyframe(Codec::H264, H264_P_AU));
    }

    #[test]
    fn h264_codec_string_from_sps() {
        let sps = find_sps(Codec::H264, H264_IDR_AU).unwrap();
        assert_eq!(h264_codec_string(sps).as_deref(), Some("avc1.64002a"));
    }

    #[test]
    fn h265_keyframes() {
        // VPS(32), SPS(33), PPS(34), IDR_W_RADL(19): header byte = type << 1
        let au = [
            0,
            0,
            0,
            1,
            32 << 1,
            1,
            0,
            0,
            1,
            33 << 1,
            1,
            0,
            0,
            1,
            34 << 1,
            1,
            0,
            0,
            1,
            19 << 1,
            1,
            0xaf,
        ];
        assert!(is_keyframe(Codec::H265, &au));
        assert!(find_sps(Codec::H265, &au).is_some());
        let trail = [0, 0, 0, 1, 1 << 1, 1, 0xd0]; // TRAIL_R
        assert!(!is_keyframe(Codec::H265, &trail));
    }
}
