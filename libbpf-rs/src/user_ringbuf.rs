use libc::E2BIG;
use libc::ENOSPC;
use std::marker::PhantomData;
use std::os::fd::AsFd;
use std::os::fd::AsRawFd;
use std::os::raw::c_uint;
use std::os::raw::c_void;
use std::ptr::null_mut;
use std::ptr::NonNull;

use crate::AsRawLibbpf;
use crate::Error;
use crate::MapHandle;
use crate::MapType;
use crate::Result;

/// A mutable reference to a type `T` sample within a [`UserRingBuffer`].
#[derive(Debug)]
pub struct UserRingBufferSample<'slf, T> {
    // A non-null pointer to data of type `T` within the ring buffer.
    ptr: NonNull<T>,

    // `PhantomData<T>` signals to the Rust compiler the ownership and variance
    // of `T`. This ensures that `UserRingBufferSample` complies with Rust’s
    // safety rules regarding lifetimes and drop semantics, despite not storing
    // `T` directly.
    _marker: PhantomData<T>,

    // Reference to the owning ring buffer. This is used to discard the sample
    // if it is not submitted before being dropped.
    rb: &'slf UserRingBuffer,

    // Track whether the sample has been submitted.
    submitted: bool,
}

impl<T> UserRingBufferSample<'_, T> {
    /// Retrieve a mutable reference to the of type `T` within the ring buffer.
    ///
    /// You can use this method to modify the data within the ring buffer prior
    /// to submitting the sample.
    ///
    /// # Examples
    /// ```
    /// struct MyStruct {
    ///    field: i32,
    /// }
    ///
    /// let data_ref: &mut MyStruct = sample.as_mut();
    /// data_ref.field = 42;
    /// user_ringbuf.submit(sample);
    /// ```
    pub fn as_mut(&mut self) -> &mut T {
        unsafe { &mut *(self.ptr.as_ptr() as *mut T) }
    }
}

impl<T> Drop for UserRingBufferSample<'_, T> {
    fn drop(&mut self) {
        // If the sample has not been submitted, explicitely discard it.
        // This is necessary to avoid leaking ring buffer memory.
        if !self.submitted {
            unsafe {
                libbpf_sys::user_ring_buffer__discard(
                    self.rb.ptr.as_ptr(),
                    self.ptr.as_ptr() as *mut _,
                );
            }
        }
    }
}

/// Represents a user ring buffer. This is a special kind of map that is used to
/// transfer data between user space and kernel space.
#[derive(Debug)]
pub struct UserRingBuffer {
    // A non-null pointer to the underlying user ring buffer.
    ptr: NonNull<libbpf_sys::user_ring_buffer>,
}

impl UserRingBuffer {
    /// Create a new user ring buffer from a map.
    ///
    /// # Errors
    /// * If the map is not a user ring buffer.
    /// * If the underlying libbpf function fails.
    pub fn new(map: &MapHandle) -> Result<Self> {
        if map.map_type() != MapType::UserRingBuf {
            return Err(Error::with_invalid_data("Must use a UserRingBuf map"));
        }

        let fd = map.as_fd();
        let raw_ptr = unsafe { libbpf_sys::user_ring_buffer__new(fd.as_raw_fd(), null_mut()) };

        let ptr = NonNull::new(raw_ptr).ok_or_else(|| {
            // Safely get the last OS error after a failed call to user_ring_buffer__new
            let errno = unsafe { *libc::__errno_location() };
            Error::from_raw_os_error(errno)
        })?;

        Ok(UserRingBuffer { ptr })
    }

    /// Reserve space in the ring buffer for a sample of type `T``.
    ///
    /// Returns a [`UserRingBufferSample`] that contains a mutable reference to
    /// a type T that can be written to. The sample must be submitted via
    /// [`UserRingBuffer::submit`](UserRingBuffer::submit) before it is dropped.
    ///
    /// This function is *not* thread-safe. It is necessary to synchronize
    /// amongst multiple producers when invoking this function.
    pub fn reserve<T>(&self) -> Result<UserRingBufferSample<'_, T>>
    where
        T: Sized,
    {
        let size = std::mem::size_of::<T>();

        let sample_ptr =
            unsafe { libbpf_sys::user_ring_buffer__reserve(self.ptr.as_ptr(), size as c_uint) }
                as *mut T;

        let ptr = NonNull::new(sample_ptr).ok_or_else(|| {
            // Fetch the current value of errno to determine the type of error.
            let errno = unsafe { *libc::__errno_location() };
            match errno {
                E2BIG => Error::with_invalid_data("Requested size is too large"),
                ENOSPC => Error::with_invalid_data("Not enough space in the ring buffer"),
                _ => Error::from_raw_os_error(errno),
            }
        })?;

        Ok(UserRingBufferSample {
            ptr,
            _marker: PhantomData,
            submitted: false,
            rb: &self,
        })
    }

    /// Submit a sample to the ring buffer.
    ///
    /// This function takes ownership of the sample and submits it to the ring
    /// buffer. After submission, the consumer will be able to read the sample
    /// from the ring buffer.
    ///
    /// This function is thread-safe. It is *not* necessary to synchronize
    /// amongst multiple producers when invoking this function.
    pub fn submit<T>(&self, mut sample: UserRingBufferSample<'_, T>) -> Result<()> {
        unsafe {
            libbpf_sys::user_ring_buffer__submit(
                self.ptr.as_ptr(),
                sample.ptr.as_ptr() as *mut c_void,
            );
        }

        sample.submitted = true;

        // The libbpf function does not return an error code, so we cannot
        // determine if the submission was successful. We assume that the
        // submission was successful if the function returns without error.
        Ok(())
    }
}

impl AsRawLibbpf for UserRingBuffer {
    type LibbpfType = libbpf_sys::user_ring_buffer;

    /// Retrieve the underlying [`libbpf_sys::user_ring_buffer`].
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
