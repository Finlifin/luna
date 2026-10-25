//! Entry diagnostics for widths that cannot use a C varargs integer argument.
use super::bits;
pub(super) fn buffer_size(width: u32) -> usize {
    // 0.30103 is an upper bound for log10(2); reserve sign and terminator.
    (u64::from(width) * 30103).div_ceil(100000) as usize + 2
}
pub(super) fn definition(width: u32, fuel: &mut usize) -> Result<String, String> {
    if !(65..=bits::MAX_BIT_WIDTH).contains(&width) {
        return Err("host decimal width is outside the representation budget".into());
    }
    let count = width.div_ceil(32);
    *fuel = fuel
        .checked_sub(count as usize)
        .ok_or("host decimal adapter budget exhausted")?;
    let mut body = format!(
        "define internal ptr @flurry_decimal_i{width}(i{width} %value, i1 %signed, ptr %buffer) {{\nentry:\n  %below = icmp slt i{width} %value, 0\n  %negative = and i1 %signed, %below\n  %negated = sub i{width} 0, %value\n  %magnitude = select i1 %negative, i{width} %negated, i{width} %value\n  %limbs = alloca [{count} x i32]\n"
    );
    for index in 0..count {
        body.push_str(&format!("  %shift{index} = lshr i{width} %magnitude, {}\n  %word{index} = trunc i{width} %shift{index} to i32\n  %slot{index} = getelementptr [{count} x i32], ptr %limbs, i32 0, i32 {index}\n  store i32 %word{index}, ptr %slot{index}\n",index*32));
    }
    body.push_str(&format!("  %end = getelementptr i8, ptr %buffer, i32 {}\n  %text = call ptr @flurry_decimal_limbs(ptr %limbs, i32 {count}, ptr %end, i1 %negative)\n  ret ptr %text\n}}\n",buffer_size(width)-1));
    Ok(body)
}
