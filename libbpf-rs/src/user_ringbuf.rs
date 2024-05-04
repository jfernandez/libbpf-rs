use crate::Result;
use crate::{AsRawLibbpf, Error, MapHandle};
use libc::{self, E2BIG, ENOSPC};
use std::os::fd::{AsFd, AsRawFd};
use std::os::raw::{c_uint, c_void};
use std::ptr::{null_mut, NonNull};

#[derive(Debug)]
pub struct UserRingBuffer {
    ptr: NonNull<libbpf_sys::user_ring_buffer>,
}

impl UserRingBuffer {
    pub fn new(map: &MapHandle) -> Result<Self> {
        let fd = map.as_fd();

        let urbuf_ptr = unsafe { libbpf_sys::user_ring_buffer__new(fd.as_raw_fd(), null_mut()) };

        let ptr = NonNull::new(urbuf_ptr).ok_or_else(|| {
            // Safely get the last OS error after a failed call to user_ring_buffer__new
            let errno = unsafe { *libc::__errno_location() };
            Error::from_raw_os_error(errno)
        })?;

        Ok(UserRingBuffer { ptr })
    }

    pub fn reserve<T>(&self) -> Result<&mut T>
    where
        T: Sized,
    {
        let size = std::mem::size_of::<T>();

        let sample_ptr =
            unsafe { libbpf_sys::user_ring_buffer__reserve(self.ptr.as_ptr(), size as c_uint) };

        NonNull::new(sample_ptr as *mut T).map_or_else(
            || {
                // Fetch the current value of errno to determine the type of error.
                let errno = unsafe { *libc::__errno_location() };
                match errno {
                    E2BIG => Err(Error::with_invalid_data("Requested size is too large")),
                    ENOSPC => Err(Error::with_invalid_data(
                        "Not enough space in the ring buffer",
                    )),
                    _ => Err(Error::from_raw_os_error(errno)),
                }
            },
            |nn_ptr| {
                // Safely return a mutable reference to the type T.
                Ok(unsafe { &mut *nn_ptr.as_ptr() })
            },
        )
    }

    pub fn submit<T>(&self, sample: &mut T) -> Result<()>{
        unsafe {
            libbpf_sys::user_ring_buffer__submit(self.ptr.as_ptr(), sample as *mut T as *mut c_void);
        }
        let errno = unsafe { *libc::__errno_location() };
        if errno != 0 {
            // Return an error if errno is set
            return Err(Error::from_raw_os_error(errno));
        }

        // Return Ok if no errors occurred
        Ok(())
    }
}

impl AsRawLibbpf for UserRingBuffer {
    type LibbpfType = libbpf_sys::user_ring_buffer;

    fn as_libbpf_object(&self) -> NonNull<Self::LibbpfType> {
        self.ptr
    }
}

impl Drop for UserRingBuffer {
    fn drop(&mut self) {
        unsafe {
            libbpf_sys::user_ring_buffer__free(self.ptr.as_ptr());
        }
    }
}
