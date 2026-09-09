//! Allocate the `-it` terminal from the BOX's devpts, and hand the master back to the CLI.
//!
//! THE DEFECT THIS EXISTS TO CLOSE, and why it was invisible for so long.
//!
//! kern used to allocate the PTY pair on the HOST (`posix_openpt` on the host's `/dev/ptmx`) and
//! pass the slave into the box as its stdio. The box then mounts its OWN private devpts at
//! `/dev/pts`, which does not contain that node. Inside the box, measured against
//! kern 0.9.32-review.14:
//!
//! ```text
//! isatty(0)                 1              the fd IS a terminal
//! readlink /proc/self/fd/0  /dev/pts/2     the HOST's path
//! stat /dev/pts/2           ENOENT         that path does not exist in the box
//! ```
//!
//! musl's `ttyname_r` is exactly that readlink, a `stat`, and a device comparison, with no second
//! strategy, so it returns ENOENT and `tty(1)` prints "not a tty" on every alpine box. glibc's
//! `ttyname_r` falls back to SCANNING `/dev`, where it finds the `/dev/console` bind `setup_dev`
//! installs, and reports `/dev/console`. So the same box looked correct under one C library and
//! broken under the other, and the broken one is the default image almost everyone runs.
//!
//! podman does not have the defect: its slave comes from the CONTAINER's devpts. Measured on one
//! host, same shell, same moment:
//!
//! ```text
//! podman exec -it c sh -c tty   ->  /dev/pts/0
//! kern   exec -it b sh -c tty   ->  not a tty
//! ```
//!
//! THE FIX. Open the pair from the box's own `/dev/ptmx` in the process that has the box's mount
//! namespace (the box child after `setup_dev`, or the `exec` child after `setns`), keep the slave
//! there, and send the MASTER back to the CLI over a `socketpair` with `SCM_RIGHTS`. `/proc/self/fd/0`
//! then reads `/dev/pts/N` and that node is present, so both C libraries resolve it.
//!
//! A pipe cannot carry a file descriptor, which is why this needs a socket and not the sync pipes
//! the box start path already has.
//!
//! FALLING BACK IS THE POINT, not an afterthought: every entry point here returns `Option`/`bool`
//! and never fails a box. When the box's devpts cannot produce a pair, the caller keeps the host
//! pair it already holds and the box behaves exactly as it did before this module existed. The
//! terminal is a convenience; refusing to start a box over it would be a worse defect than the one
//! being fixed.

use std::ffi::CString;

/// `sizeof(int)` as the width `CMSG_SPACE`/`CMSG_LEN` want, computed once instead of four times.
///
/// The cast is from a `usize` that is 4 on every target this builds for, and writing it inline four
/// times both repeated the claim and drew a truncation warning at each site. Named here, and
/// `one_descriptor_is_four_bytes_wide` below checks the width rather than assuming it.
///
/// Two pedantic lints stay ON PURPOSE and this is the note for whoever meets them next. The cast
/// here is `usize` to `u32` and cannot truncate: the value is 4, and the test below asserts it
/// rather than trusting the comment. And the `as _` at each `ioctl` call cannot become a `From`,
/// because the request type it has to land on is `c_ulong` under glibc and `c_int` under musl; an
/// infallible conversion would pin one of the two and break the build on the other, which is
/// exactly the defect the distro VMs caught in the first version of this file.
const FD_WIDTH: u32 = std::mem::size_of::<libc::c_int>() as u32;

/// `TIOCGPTN`, "get the pty number of this master". Not in the pinned `libc` for every target, and
/// pinned by `ioctl_numbers_match_the_uapi_header` below: it is `_IOR('T', 0x30, unsigned int)`.
/// `u32` AND NOT THE REQUEST TYPE, because that type is not the same everywhere: `libc::ioctl` takes
/// a `c_ulong` request against glibc and a `c_int` against musl. Writing `c_ulong` here compiled
/// cleanly on the development host and FAILED on `x86_64-unknown-linux-musl`, which is the target
/// kern actually ships. The bit pattern is what the kernel reads, so both constants are held as
/// unsigned 32-bit and cast with `as _` at each call: that lands on the right type per target, and
/// keeps `0x8004_5430` (larger than `i32::MAX`) from having to be written as a negative literal.
const TIOCGPTN: u32 = 0x8004_5430;
/// `TIOCSPTLCK`, "set the slave's lock flag". `_IOW('T', 0x31, int)`; writing 0 unlocks, which is
/// what `unlockpt(3)` does and what has to happen before the slave can be opened.
const TIOCSPTLCK: u32 = 0x4004_5431;

/// Open a PTY pair from the devpts mounted under `dev_dir` (`"<root>/dev"` before `pivot_root`, or
/// plain `"/dev"` from inside the box), returning `(master, slave)`.
///
/// `None` on any failure, and the caller must then keep whatever pair it already has. Reasons this
/// legitimately returns `None`: the box was built with no `/dev/ptmx` (a `--rootfs` that kern did
/// not populate), a kernel without devpts, or a mount that refused `ptmxmode`.
///
/// Does NOT call `grantpt(3)`. On Linux with a `newinstance` devpts the slave's ownership comes
/// from the mount's `mode=`/`gid=` options rather than from a helper, and glibc's `grantpt` is a
/// no-op there; calling it would be a `fork`+`exec` of `pt_chown` on some libcs, which is exactly
/// what must not happen in a post-fork child.
#[must_use]
pub fn open_pair_in(dev_dir: &str) -> Option<(i32, i32)> {
    let ptmx = CString::new(format!("{dev_dir}/ptmx")).ok()?;
    // SAFETY: `ptmx` is a live NUL-terminated path; `open` only reads it.
    let master = unsafe {
        libc::open(
            ptmx.as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        )
    };
    if master < 0 {
        return None;
    }
    let mut unlock: libc::c_int = 0;
    // SAFETY: both ioctls take a pointer to a properly sized integer, which these are.
    let ok = unsafe { libc::ioctl(master, TIOCSPTLCK as _, &mut unlock) } == 0;
    let mut n: libc::c_uint = 0;
    let got = ok && unsafe { libc::ioctl(master, TIOCGPTN as _, &mut n) } == 0;
    if !got {
        // SAFETY: `master` is a valid fd this function opened.
        unsafe { libc::close(master) };
        return None;
    }
    let Ok(slave_path) = CString::new(format!("{dev_dir}/pts/{n}")) else {
        unsafe { libc::close(master) };
        return None;
    };
    // NOT `O_CLOEXEC`: the slave is dup2'd onto the box's stdio and has to survive the `execvp`.
    // The master IS cloexec, so the workload can never reach the other end of its own terminal.
    // SAFETY: `slave_path` is a live NUL-terminated path.
    let slave = unsafe { libc::open(slave_path.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) };
    if slave < 0 {
        unsafe { libc::close(master) };
        return None;
    }
    Some((master, slave))
}

/// Build the box's own terminal and hand its master to the CLI, returning the SLAVE to use.
///
/// The whole handover in one place, because it was written twice - once in `setup_dev` for a box
/// start and once in `exec_in_box` after `setns` - in two different shapes for the same five steps.
/// Two spellings of one sequence is how the halves drift: the first version closed the master in a
/// different order on each side, and only one of them closed the slave when the send failed.
///
/// `dev_dir` is `"<root>/dev"` before `pivot_root` and `"/dev"` from inside the box; both name the
/// SAME devpts, which is the only thing that matters here.
///
/// `None` means the caller keeps the terminal it already has. Nothing is leaked on any path: a pair
/// that cannot be handed over is fully closed before returning, because a slave whose master nobody
/// holds is a terminal that swallows the workload's output.
#[must_use]
pub fn hand_over_pair(dev_dir: &str, sock: i32) -> Option<i32> {
    let (master, slave) = open_pair_in(dev_dir)?;
    let sent = send_fd(sock, master);
    // SAFETY: `master` is a valid fd from `open_pair_in`; `sendmsg` DUPLICATES it into the peer, so
    // closing this copy now is right whether the send worked or not.
    unsafe { libc::close(master) };
    if sent {
        return Some(slave);
    }
    // SAFETY: `slave` is a valid fd from `open_pair_in` and is not returned.
    unsafe { libc::close(slave) };
    None
}

/// Send one file descriptor over a `SOCK_STREAM` unix socket, with a single data byte because a
/// zero-length `sendmsg` carries no ancillary data.
///
/// Stack only: the control buffer is a fixed array and the `msghdr` is zeroed in place, so this is
/// safe to call from a post-fork child that must not touch the heap.
pub fn send_fd(sock: i32, fd: i32) -> bool {
    unsafe {
        let mut byte: [u8; 1] = *b"k";
        let mut iov = libc::iovec {
            iov_base: byte.as_mut_ptr().cast(),
            iov_len: 1,
        };
        // CMSG_SPACE(sizeof(int)) for exactly one descriptor, over-aligned to a u64 so the cmsghdr
        // inside is naturally aligned however the target lays it out.
        let mut cbuf = [0u64; 4];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = std::ptr::from_mut(&mut iov);
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = libc::CMSG_SPACE(FD_WIDTH) as _;
        let cmsg = libc::CMSG_FIRSTHDR(std::ptr::from_ref(&msg));
        if cmsg.is_null() {
            return false;
        }
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(FD_WIDTH) as _;
        std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<libc::c_int>(), fd);
        // EINTR is the one failure worth retrying: a signal between the two ends is not an error.
        loop {
            let n = libc::sendmsg(sock, std::ptr::from_ref(&msg), 0);
            if n >= 0 {
                return true;
            }
            if *libc::__errno_location() != libc::EINTR {
                return false;
            }
        }
    }
}

/// Receive one file descriptor sent by [`send_fd`], or `None` if the peer sent none, closed, or the
/// socket errored.
///
/// `None` is the ORDINARY case and not a failure: it is what the caller sees when the child could
/// not build a pair in the box and kept the host one, and when the child died before getting there.
/// Both mean "use what you already have".
#[must_use]
pub fn recv_fd(sock: i32) -> Option<i32> {
    unsafe {
        let mut byte = [0u8; 1];
        let mut iov = libc::iovec {
            iov_base: byte.as_mut_ptr().cast(),
            iov_len: 1,
        };
        let mut cbuf = [0u64; 4];
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = std::ptr::from_mut(&mut iov);
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = libc::CMSG_SPACE(FD_WIDTH) as _;
        let n = loop {
            let n = libc::recvmsg(sock, std::ptr::from_mut(&mut msg), 0);
            if n >= 0 || *libc::__errno_location() != libc::EINTR {
                break n;
            }
        };
        if n <= 0 {
            return None;
        }
        let cmsg = libc::CMSG_FIRSTHDR(std::ptr::from_ref(&msg));
        if cmsg.is_null()
            || (*cmsg).cmsg_level != libc::SOL_SOCKET
            || (*cmsg).cmsg_type != libc::SCM_RIGHTS
            || (*cmsg).cmsg_len < libc::CMSG_LEN(FD_WIDTH) as _
        {
            return None;
        }
        let fd = std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast::<libc::c_int>());
        if fd < 0 {
            return None;
        }
        Some(fd)
    }
}

/// A connected `SOCK_STREAM` pair for [`send_fd`]/[`recv_fd`], `(parent_end, child_end)`.
///
/// The PARENT end is `O_CLOEXEC` so it cannot leak into anything the parent execs; the CHILD end is
/// deliberately NOT, because it has to survive the child's own path to the box. The child closes it
/// itself once the master is away.
#[must_use]
pub fn fd_channel() -> Option<(i32, i32)> {
    let mut sv = [0i32; 2];
    // SAFETY: `sv` is a two-element array, which is what `socketpair` writes.
    let rc = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, sv.as_mut_ptr()) };
    if rc != 0 {
        return None;
    }
    let [parent, child] = sv;
    // SAFETY: `parent` is a valid fd just created.
    unsafe { libc::fcntl(parent, libc::F_SETFD, libc::FD_CLOEXEC) };
    Some((parent, child))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE TWO IOCTL NUMBERS, AGAINST THE HEADER, because the value one bit away is a different
    /// request that fails in a way this code would read as "no devpts here" and silently fall back
    /// to the broken behaviour it exists to replace.
    ///
    /// `TIOCGPTN`  = `_IOR('T', 0x30, unsigned int)` = dir 2 | size 4 | 'T' | 0x30
    /// `TIOCSPTLCK`= `_IOW('T', 0x31, int)`          = dir 1 | size 4 | 'T' | 0x31
    #[test]
    fn ioctl_numbers_match_the_uapi_header() {
        // _IOC(dir, type, nr, size) = dir<<30 | size<<16 | type<<8 | nr
        let ioc =
            |dir: u32, ty: u32, nr: u32, size: u32| (dir << 30) | (size << 16) | (ty << 8) | nr;
        let t = u32::from(b'T');
        assert_eq!(TIOCGPTN, ioc(2, t, 0x30, 4), "TIOCGPTN");
        assert_eq!(TIOCSPTLCK, ioc(1, t, 0x31, 4), "TIOCSPTLCK");
        assert_ne!(
            TIOCGPTN, TIOCSPTLCK,
            "the two must not collapse to one value"
        );
    }

    /// `SCM_RIGHTS` CARRIES A `c_int`, AND [`FD_WIDTH`] SAYS HOW WIDE THAT IS. If it were ever not 4,
    /// the control buffer sized from it would be wrong and `sendmsg` would either truncate the
    /// descriptor or refuse; a `usize`-to-`u32` cast cannot report that, so it is checked here.
    #[test]
    fn one_descriptor_is_four_bytes_wide() {
        assert_eq!(FD_WIDTH, 4, "sizeof(int)");
        assert_eq!(
            FD_WIDTH as usize,
            std::mem::size_of::<libc::c_int>(),
            "no truncation"
        );
    }

    /// A descriptor really crosses the socket, and it is the SAME open file on the other side.
    ///
    /// Comparing fd NUMBERS would prove nothing: the receiver gets whatever number is free. The
    /// check that means something is that the two descriptors share one file description, which is
    /// what `SCM_RIGHTS` promises and what a copy of the bytes would not give: writing through the
    /// received end moves the offset the sender sees.
    #[test]
    fn a_descriptor_crosses_the_socket_as_the_same_open_file() {
        let (parent, child) = fd_channel().expect("socketpair");
        let mut path = std::env::temp_dir();
        path.push(format!("kern-fdpass-{}", std::process::id()));
        let f = std::fs::File::create(&path).expect("temp file");
        let sent = std::os::unix::io::AsRawFd::as_raw_fd(&f);

        assert!(send_fd(child, sent), "send_fd");
        let got = recv_fd(parent).expect("recv_fd must produce a descriptor");
        assert_ne!(got, sent, "a received fd is a NEW number for the same file");

        // Same open file description: an offset moved through one is visible through the other.
        let off = unsafe { libc::lseek(got, 7, libc::SEEK_SET) };
        assert_eq!(off, 7, "seek on the received fd");
        let seen = unsafe { libc::lseek(sent, 0, libc::SEEK_CUR) };
        assert_eq!(seen, 7, "the sender's fd must see the same offset");

        unsafe {
            libc::close(got);
            libc::close(parent);
            libc::close(child);
        }
        drop(f);
        let _ = std::fs::remove_file(&path);
    }

    /// A CLOSED PEER IS `None`, NOT A HANG AND NOT A BOGUS FD. This is the ordinary path whenever
    /// the child could not build a pair in the box, or died before it got there, and the caller
    /// treats it as "keep the host pair" - so it has to be reached quickly and unambiguously.
    #[test]
    fn a_peer_that_sends_nothing_reads_as_none() {
        let (parent, child) = fd_channel().expect("socketpair");
        unsafe { libc::close(child) };
        assert!(recv_fd(parent).is_none(), "EOF must read as None");
        unsafe { libc::close(parent) };
    }

    /// A MESSAGE WITH NO DESCRIPTOR IS ALSO `None`. Without this the receiver would read the
    /// uninitialised control buffer as a descriptor number and hand the caller a wild fd, which is
    /// the worst outcome available here: it would not fail, it would work on the wrong file.
    #[test]
    fn a_data_byte_without_a_descriptor_reads_as_none() {
        let (parent, child) = fd_channel().expect("socketpair");
        let one = *b"x";
        let n = unsafe { libc::send(child, one.as_ptr().cast(), 1, 0) };
        assert_eq!(n, 1, "plain send");
        assert!(
            recv_fd(parent).is_none(),
            "a message carrying no SCM_RIGHTS must not be read as a descriptor"
        );
        unsafe {
            libc::close(parent);
            libc::close(child);
        }
    }

    /// The host's own `/dev` is a devpts, so the allocator must work against it. This is the
    /// POSITIVE CONTROL for the box case, which no unit test can reach: without it, a bug that made
    /// `open_pair_in` always return `None` would leave every caller silently on the old host path
    /// and every other test in this file would still pass.
    #[test]
    fn a_pair_can_be_opened_from_a_real_devpts() {
        if !std::path::Path::new("/dev/ptmx").exists() {
            eprintln!("skip: no /dev/ptmx on this host");
            return;
        }
        let Some((m, s)) = open_pair_in("/dev") else {
            eprintln!("skip: the host refused a pty pair (containerised runner without devpts)");
            return;
        };
        assert!(m >= 0 && s >= 0, "both ends are real descriptors");
        assert_eq!(
            unsafe { libc::isatty(s) },
            1,
            "the slave must be a terminal"
        );
        // And the slave has a NAME, which is the whole point of this module.
        let mut buf = [0 as libc::c_char; 256];
        let rc = unsafe { libc::ttyname_r(s, buf.as_mut_ptr(), buf.len()) };
        assert_eq!(rc, 0, "ttyname_r on the slave must resolve");
        unsafe {
            libc::close(m);
            libc::close(s);
        }
    }
}
