//! In-process hardware counters: one `perf_event_open` group (cycles leader, instructions,
//! branch misses) for the calling thread, user space only, enabled once at open.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reading {
    pub cycles: u64,
    pub instructions: u64,
    pub branch_misses: u64,
    pub time_enabled: u64,
    pub time_running: u64,
}

/// `struct perf_event_attr` up to `PERF_ATTR_SIZE_VER5` (112 bytes).
#[repr(C)]
#[derive(Default)]
struct PerfEventAttr {
    type_: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    bp_addr: u64,
    bp_len: u64,
    branch_sample_type: u64,
    sample_regs_user: u64,
    sample_stack_user: u32,
    clockid: i32,
    sample_regs_intr: u64,
    aux_watermark: u32,
    sample_max_stack: u16,
    reserved_2: u16,
}

const PERF_TYPE_HARDWARE: u32 = 0;
const HW_CPU_CYCLES: u64 = 0;
const HW_INSTRUCTIONS: u64 = 1;
const HW_BRANCH_MISSES: u64 = 5;
const FORMAT_TOTAL_TIME_ENABLED: u64 = 1;
const FORMAT_TOTAL_TIME_RUNNING: u64 = 2;
const FORMAT_GROUP: u64 = 8;
const FLAG_DISABLED: u64 = 1 << 0;
const FLAG_EXCLUDE_KERNEL: u64 = 1 << 5;
const FLAG_EXCLUDE_HV: u64 = 1 << 6;
const IOC_ENABLE: libc::c_ulong = 0x2400;
const IOC_FLAG_GROUP: libc::c_ulong = 1;

pub struct Counters {
    fds: [i32; 3],
}

fn open_one(config: u64, group_fd: i32, leader: bool) -> Result<i32, String> {
    let mut a = PerfEventAttr::default();
    a.type_ = PERF_TYPE_HARDWARE;
    a.size = core::mem::size_of::<PerfEventAttr>() as u32;
    a.config = config;
    a.read_format = FORMAT_GROUP | FORMAT_TOTAL_TIME_ENABLED | FORMAT_TOTAL_TIME_RUNNING;
    a.flags = FLAG_EXCLUDE_KERNEL | FLAG_EXCLUDE_HV | if leader { FLAG_DISABLED } else { 0 };
    // pid = 0 (calling thread), cpu = -1 (any)
    let fd = unsafe { libc::syscall(libc::SYS_perf_event_open, &a as *const PerfEventAttr, 0, -1, group_fd, 0u64) };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        return Err(format!("perf_event_open(config={config}) failed: {e}"));
    }
    Ok(fd as i32)
}

impl Counters {
    /// Opens the group for the calling thread and enables it. Errors are stop-gate 3.
    pub fn open() -> Result<Counters, String> {
        let leader = open_one(HW_CPU_CYCLES, -1, true)?;
        let close_on_err = |fds: &[i32], e: String| {
            for &f in fds {
                unsafe { libc::close(f) };
            }
            e
        };
        let ins = open_one(HW_INSTRUCTIONS, leader, false).map_err(|e| close_on_err(&[leader], e))?;
        let br = open_one(HW_BRANCH_MISSES, leader, false).map_err(|e| close_on_err(&[leader, ins], e))?;
        let r = unsafe { libc::ioctl(leader, IOC_ENABLE, IOC_FLAG_GROUP) };
        if r < 0 {
            let e = std::io::Error::last_os_error();
            return Err(close_on_err(&[leader, ins, br], format!("PERF_EVENT_IOC_ENABLE failed: {e}")));
        }
        Ok(Counters { fds: [leader, ins, br] })
    }

    /// Reads the whole group with one `read(2)`. Must be called on the thread that opened it.
    pub fn read(&self) -> Reading {
        // { nr, time_enabled, time_running, values[nr] }
        let mut buf = [0u64; 6];
        let n = unsafe { libc::read(self.fds[0], buf.as_mut_ptr() as *mut libc::c_void, core::mem::size_of_val(&buf)) };
        assert!(n == core::mem::size_of_val(&buf) as isize && buf[0] == 3, "short or malformed perf group read ({n} bytes)");
        Reading { time_enabled: buf[1], time_running: buf[2], cycles: buf[3], instructions: buf[4], branch_misses: buf[5] }
    }
}

impl Drop for Counters {
    fn drop(&mut self) {
        for &f in &self.fds {
            unsafe { libc::close(f) };
        }
    }
}
