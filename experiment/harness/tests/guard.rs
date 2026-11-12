mod support;
use msx_harness::guard::{GuardBuf, OutSlot};

#[test]
fn place_end_reads_back_and_faults_after() {
    let mut g = GuardBuf::new(16);
    let data = [1u8, 2, 3, 4, 5];
    let p = g.place_end(&data);
    let back: Vec<u8> = (0..5).map(|i| unsafe { p.add(i).read_volatile() }).collect();
    assert_eq!(back, data);
    let sig = support::signal_of_child(|| unsafe {
        let _ = p.add(5).read_volatile();
    });
    assert_eq!(sig, Some(libc::SIGSEGV));
}

#[test]
fn place_start_faults_before() {
    let mut g = GuardBuf::new(16);
    let data = [9u8; 5];
    let p = g.place_start(&data);
    assert_eq!(unsafe { p.read_volatile() }, 9);
    let sig = support::signal_of_child(|| unsafe {
        let _ = p.sub(1).read_volatile();
    });
    assert_eq!(sig, Some(libc::SIGSEGV));
}

#[test]
fn large_buffer_spans_pages() {
    let mut g = GuardBuf::new(10_000);
    let data: Vec<u8> = (0..10_000).map(|i| i as u8).collect();
    let p = g.place_end(&data);
    let s = unsafe { core::slice::from_raw_parts(p, data.len()) };
    assert_eq!(s, &data[..]);
}

#[test]
fn out_slot_detects_writes_and_overrun() {
    let mut s = OutSlot::new(1);
    let p = s.arm();
    assert_eq!(s.take(), ([msx_harness::guard::SENTINEL, 0, 0], false));
    unsafe { p.write_volatile(0x1234) };
    assert_eq!(s.take(), ([0x1234, 0, 0], true));
    let sig = support::signal_of_child(|| unsafe { p.add(1).write_volatile(1) });
    assert_eq!(sig, Some(libc::SIGSEGV));
}
