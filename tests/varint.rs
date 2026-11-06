use rime_counter::util::read_varint;

#[test]
fn decodes_single_byte() {
    let mut pos = 0;
    assert_eq!(read_varint(&[0x00], &mut pos), Some(0));
    assert_eq!(pos, 1);

    let mut pos = 0;
    assert_eq!(read_varint(&[0x7f], &mut pos), Some(127));
    assert_eq!(pos, 1);
}

#[test]
fn decodes_multi_byte() {
    // 200 = 0xC8 0x01
    let mut pos = 0;
    assert_eq!(read_varint(&[0xc8, 0x01], &mut pos), Some(200));
    assert_eq!(pos, 2);
}

#[test]
fn reads_from_offset() {
    let buf = [0xff, 0x05];
    let mut pos = 1;
    assert_eq!(read_varint(&buf, &mut pos), Some(5));
    assert_eq!(pos, 2);
}

#[test]
fn returns_none_when_truncated() {
    // 只有续位没有结束字节
    let mut pos = 0;
    assert_eq!(read_varint(&[0x80], &mut pos), None);
    assert_eq!(read_varint(&[], &mut pos), None);
}

#[test]
fn rejects_overlong_encoding() {
    let buf = [0x80; 11];
    let mut pos = 0;
    assert_eq!(read_varint(&buf, &mut pos), None);
}