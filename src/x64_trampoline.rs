//! x64 位置无关 trampoline 机器码生成器。
//!
//! 为 SMBIOS 补丁生成 trampoline：调用原 `GetSystemFirmwareTable`（通过原始 IAT）
//! 后在 RSMB buffer 内按预计算偏移覆写字符串字段。
//!
//! trampoline 通过 RIP-相对 `lea rax,[rip+disp] + mov rax,[rax]` 读 IAT 条目。
//! 该 disp32 的字节偏移通过返回值上报，调用方在 append 节后原地修正即可，
//! 无需两遍构建。

pub struct ReplaceEntry {
    pub offset: u32,
    pub verify: Vec<u8>,
    pub replace: Vec<u8>,
}

pub struct TrampolineCode {
    /// 可直接写入节的完整机器码。
    pub bytes: Vec<u8>,
    /// bytes 中 IAT 引用 disp32 的字节偏移（4 字节小端）。
    /// 调用方拿到节的真实 RVA 后应将其修正为：
    ///   disp = iat_rva - (actual_trampoline_rva + iat_disp_byte_offset + 7)
    pub iat_disp_byte_offset: usize,
}

pub fn build_trampoline(iat_rva: u32, entries: &[ReplaceEntry]) -> TrampolineCode {
    assert!(!entries.is_empty());

    let mut code = Vec::new();

    // =========================================================
    // Prologue: push rbx/rsi/rdi/r12/r13 ; sub rsp,0x28
    // =========================================================
    code.extend_from_slice(&[0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55]);
    code.extend_from_slice(&[0x48, 0x83, 0xEC, 0x28]);
    code.extend_from_slice(&[0x48, 0x89, 0xCB]); // mov rbx, rcx  (provider)
    code.extend_from_slice(&[0x48, 0x89, 0xD6]); // mov rsi, rdx  (table)
    code.extend_from_slice(&[0x4C, 0x89, 0xC7]); // mov rdi, r8   (buf)

    // =========================================================
    // Call original through IAT:  lea rax,[rip+disp] ; mov rax,[rax] ; call rax
    // =========================================================
    let lea_pos = code.len(); // 7-byte instruction
    code.push(0x48);
    code.push(0x8D);
    code.push(0x05);
    let iat_disp_offset = code.len(); // ← caller patches these 4 bytes later
    code.extend_from_slice(&[0u8; 4]);
    code.extend_from_slice(&[0x48, 0x8B, 0x00]); // mov rax, [rax]
    code.extend_from_slice(&[0x48, 0x83, 0xEC, 0x20]); // sub rsp, 0x20
    code.extend_from_slice(&[0xFF, 0xD0]); // call rax
    code.extend_from_slice(&[0x48, 0x83, 0xC4, 0x20]); // add rsp, 0x20
    code.extend_from_slice(&[0x49, 0x89, 0xC4]); // mov r12, rax

    // 先用占位 RVA=0 计算 disp32（后续原地修正为 actual_rva）
    let placeholder_disp = (iat_rva as i64) - ((lea_pos + 7) as i64);
    code[iat_disp_offset..iat_disp_offset + 4]
        .copy_from_slice(&(placeholder_disp as i32).to_le_bytes());

    // =========================================================
    // Check: result!=NULL && provider=='RSMB' && table==0
    // =========================================================
    code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
    let jz_pos = code.len();
    code.extend_from_slice(&[0x0F, 0x84, 0, 0, 0, 0]);

    code.extend_from_slice(&[0x81, 0xFB, 0x42, 0x4D, 0x53, 0x52]); // cmp ebx,'RSMB'
    let jne1_pos = code.len();
    code.extend_from_slice(&[0x0F, 0x85, 0, 0, 0, 0]);

    code.extend_from_slice(&[0x85, 0xF6]); // test esi,esi
    let jnz_pos = code.len();
    code.extend_from_slice(&[0x0F, 0x85, 0, 0, 0, 0]);

    // =========================================================
    // Replace blocks
    // =========================================================
    let mut jmp_patches: Vec<(usize, usize)> = Vec::new();

    for entry in entries.iter() {
        let len = entry.verify.len() as u32;

        // lea rcx, [rdi + offset]
        code.push(0x48);
        code.push(0x8D);
        code.push(0x8F);
        code.extend_from_slice(&entry.offset.to_le_bytes());

        for b_idx in 0..len as usize {
            let b = entry.verify[b_idx];
            if b_idx < 128 {
                code.extend_from_slice(&[0x80, 0x79, b_idx as u8, b]);
            } else {
                code.extend_from_slice(&[0x80, 0xB9]);
                code.extend_from_slice(&(b_idx as u32).to_le_bytes());
                code.push(b);
            }
            let p = code.len();
            code.extend_from_slice(&[0x0F, 0x85, 0, 0, 0, 0]);
            jmp_patches.push((p + 2, 0)); // disp32 position = p+2
        }

        for b_idx in 0..len as usize {
            let b = entry.replace[b_idx];
            if b_idx < 128 {
                code.extend_from_slice(&[0xC6, 0x41, b_idx as u8, b]);
            } else {
                code.extend_from_slice(&[0xC6, 0x81]);
                code.extend_from_slice(&(b_idx as u32).to_le_bytes());
                code.push(b);
            }
        }

        let block_end = code.len();
        for _ in 0..len {
            let (pos, _) = jmp_patches.pop().expect("jmp_patches underflow");
            jmp_patches.push((pos, block_end));
        }
    }

    // =========================================================
    // Epilogue
    // =========================================================
    let done_label = code.len();
    code.extend_from_slice(&[0x4C, 0x89, 0xE0]); // mov rax, r12
    code.extend_from_slice(&[0x48, 0x83, 0xC4, 0x28]); // add rsp, 0x28
    code.extend_from_slice(&[0x41, 0x5D, 0x41, 0x5C, 0x5F, 0x5E, 0x5B]); // pop r13/r12/rdi/rsi/rbx
    code.push(0xC3);

    // Patch guard + replace-block jumps
    for patch_pos in [jz_pos + 2, jne1_pos + 2, jnz_pos + 2] {
        let disp = (done_label as i64) - ((patch_pos + 4) as i64);
        code[patch_pos..patch_pos + 4].copy_from_slice(&(disp as i32).to_le_bytes());
    }
    for (patch_pos, target) in &jmp_patches {
        let disp = (*target as i64) - ((patch_pos + 4) as i64);
        code[*patch_pos..*patch_pos + 4].copy_from_slice(&(disp as i32).to_le_bytes());
    }

    TrampolineCode {
        bytes: code,
        iat_disp_byte_offset: iat_disp_offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<ReplaceEntry> {
        vec![
            ReplaceEntry {
                offset: 100,
                verify: b"TUF B360M".to_vec(),
                replace: b"TM2424\0\0".to_vec(),
            },
            ReplaceEntry {
                offset: 200,
                verify: b"System manufacturer\0".to_vec(),
                replace: b"XIAOMI\0\0\0\0\0\0\0\0\0\0".to_vec(),
            },
        ]
    }

    #[test]
    fn trampoline_is_non_empty() {
        let tc = build_trampoline(0x8DF600, &fixture());
        assert!(!tc.bytes.is_empty());
        assert!(tc.bytes.len() > 100);
    }

    #[test]
    fn trampoline_starts_with_push_rbx() {
        let tc = build_trampoline(0x1000, &fixture());
        assert_eq!(tc.bytes[0], 0x53);
    }

    #[test]
    fn iat_disp_offset_is_within_range() {
        let tc = build_trampoline(0x5000, &fixture());
        assert!(tc.iat_disp_byte_offset > 0);
        assert!(tc.iat_disp_byte_offset + 4 <= tc.bytes.len());
    }

    #[test]
    fn iat_disp_encodes_target() {
        let iat_rva = 0x8DF600u32;
        let trampoline_rva = 0xE67000u32;
        let tc = build_trampoline(iat_rva, &fixture());

        // 模拟调用方修正
        let mut bytes = tc.bytes;
        let next_rip = trampoline_rva + tc.iat_disp_byte_offset as u32 + 7;
        let correct_disp = (iat_rva as i64) - (next_rip as i64);
        bytes[tc.iat_disp_byte_offset..tc.iat_disp_byte_offset + 4]
            .copy_from_slice(&(correct_disp as i32).to_le_bytes());

        let stored = i32::from_le_bytes(
            bytes[tc.iat_disp_byte_offset..tc.iat_disp_byte_offset + 4]
                .try_into()
                .unwrap(),
        );
        assert_eq!(stored, correct_disp as i32);
    }
}
