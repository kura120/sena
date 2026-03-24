use std::io::{self, Read, Seek, SeekFrom};
use std::fs::File;
use std::path::Path;

const GGUF_MAGIC: &[u8; 4] = b"GGUF";

#[derive(Debug, Clone, Default)]
pub struct GgufMetadata {
    pub name: Option<String>,
    pub architecture: Option<String>,
    pub file_type: Option<u32>,
}

/// Read a u32 from the reader in little-endian.
fn read_u32(reader: &mut impl Read) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

/// Read a u64 from the reader in little-endian.
fn read_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

/// Read a GGUF string (u64 length + bytes).
fn read_gguf_string(reader: &mut impl Read) -> io::Result<String> {
    let len = read_u64(reader)? as usize;
    if len > 1_000_000 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "String too long"));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Skip a GGUF value based on its type.
fn skip_gguf_value(reader: &mut (impl Read + Seek), value_type: u32) -> io::Result<()> {
    match value_type {
        0 | 1 | 7 => { reader.seek(SeekFrom::Current(1))?; }  // u8, i8, bool
        2 | 3 => { reader.seek(SeekFrom::Current(2))?; }      // u16, i16
        4..=6 => { reader.seek(SeekFrom::Current(4))?; }  // u32, i32, f32
        8 => { read_gguf_string(reader)?; }                     // string
        9 => {                                                    // array
            let elem_type = read_u32(reader)?;
            let count = read_u64(reader)?;
            // Cap array iteration to prevent DoS from malicious files
            if count > 1_000_000 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "Array count too large"));
            }
            for _ in 0..count {
                skip_gguf_value(reader, elem_type)?;
            }
        }
        10..=12 => { reader.seek(SeekFrom::Current(8))?; } // u64, i64, f64
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unknown value type: {}", value_type))),
    }
    Ok(())
}

/// Parse GGUF file header and extract general.* metadata.
/// Stops after finding all needed keys or after reading all KV pairs.
pub fn parse_gguf_metadata(path: &Path) -> io::Result<GgufMetadata> {
    let mut file = File::open(path)?;
    let mut metadata = GgufMetadata::default();
    
    // Read magic
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if &magic != GGUF_MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Not a GGUF file"));
    }
    
    // Read version
    let version = read_u32(&mut file)?;
    if !(2..=3).contains(&version) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Unsupported GGUF version: {}", version)));
    }
    
    // Read tensor count and metadata KV count
    let _tensor_count = read_u64(&mut file)?;
    let kv_count = read_u64(&mut file)?;
    
    // Limit to reasonable KV count to prevent OOM
    let kv_limit = kv_count.min(10000);
    
    let mut found_count = 0u32;
    
    for _ in 0..kv_limit {
        // Read key
        let key = read_gguf_string(&mut file)?;
        let value_type = read_u32(&mut file)?;
        
        match key.as_str() {
            "general.name" if value_type == 8 => {
                metadata.name = Some(read_gguf_string(&mut file)?);
                found_count += 1;
            }
            "general.architecture" if value_type == 8 => {
                metadata.architecture = Some(read_gguf_string(&mut file)?);
                found_count += 1;
            }
            "general.file_type" if value_type == 4 => {
                metadata.file_type = Some(read_u32(&mut file)?);
                found_count += 1;
            }
            _ => {
                skip_gguf_value(&mut file, value_type)?;
            }
        }
        
        // Stop early if we found all 3 keys
        if found_count >= 3 {
            break;
        }
    }
    
    Ok(metadata)
}

/// Map GGUF file_type integer to a human-readable quantization string.
pub fn file_type_to_quantization(file_type: u32) -> &'static str {
    match file_type {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q8_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "IQ3_XXS",
        22 => "IQ1_S",
        23 => "IQ4_NL",
        24 => "IQ3_S",
        25 => "IQ2_S",
        26 => "IQ4_XS",
        _ => "Unknown",
    }
}
