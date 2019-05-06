//! A pointer type for heap allocation.
//!
//! `Box<T>`, casually referred to as a 'box', provides the simplest form of
//! heap allocation in Rust. Boxes provide ownership for this allocation, and
//! drop their contents when they go out of scope.
//!
//! For non-zero-sized values, a [`Box`] will use the [`Global`] allocator for
//! its allocation. It is valid to convert both ways between a [`Box`] and a
//! raw pointer allocated with the [`Global`] allocator, given that the
//! [`Layout`] used with the allocator is correct for the type. More precisely,
//! a `value: *mut T` that has been allocated with the [`Global`] allocator
//! with `Layout::for_value(&*value)` may be converted into a box using
//! `Box::<T>::from_raw(value)`. Conversely, the memory backing a `value: *mut
//! T` obtained from `Box::<T>::into_raw` may be deallocated using the
//! [`Global`] allocator with `Layout::for_value(&*value)`.
//!
//! # Examples
//!
//! Move a value from the stack to the heap by creating a [`Box`]:
//!
//! ```
//! let val: u8 = 5;
//! let boxed: Box<u8> = Box::new(val);
//! ```
//!
//! Move a value from a [`Box`] back to the stack by [dereferencing]:
//!
//! ```
//! let boxed: Box<u8> = Box::new(5);
//! let val: u8 = *boxed;
//! ```
//!
//! Creating a recursive data structure:
//!
//! ```
//! #[derive(Debug)]
//! enum List<T> {
//!     Cons(T, Box<List<T>>),
//!     Nil,
//! }
//!
//! fn main() {
//!     let list: List<i32> = List::Cons(1, Box::new(List::Cons(2, Box::new(List::Nil))));
//!     println!("{:?}", list);
//! }
//! ```
//!
//! This will print `Cons(1, Cons(2, Nil))`.
//!
//! Recursive structures must be boxed, because if the definition of `Cons`
//! looked like this:
//!
//! ```compile_fail,E0072
//! # enum List<T> {
//! Cons(T, List<T>),
//! # }
//! ```
//!
//! It wouldn't work. This is because the size of a `List` depends on how many
//! elements are in the list, and so we don't know how much memory to allocate
//! for a `Cons`. By introducing a `Box`, which has a defined size, we know how
//! big `Cons` needs to be.
//!
//! [dereferencing]: ../../std/ops/trait.Deref.html
//! [`Box`]: struct.Box.html
//! [`Global`]: ../alloc/struct.Global.html
//! [`Layout`]: ../alloc/struct.Layout.html

#![stable(feature = "rust1", since = "1.0.0")]

use core::any::Any;
use core::borrow;
use core::cmp::Ordering;
use core::convert::From;
use core::fmt;
use core::future::Future;
use core::hash::{Hash, Hasher};
use core::iter::{Iterator, FromIterator, FusedIterator};
#[cfg(stage0)]
use core::marker::PhantomData;
use core::marker::{Unpin, Unsize};
use core::mem;
use core::pin::Pin;
use core::ops::{
    CoerceUnsized, DispatchFromDyn, Deref, DerefMut, Receiver, Generator, GeneratorState
};
use core::ptr::{self, NonNull, Unique};
use core::task::{Context, Poll};

use crate::abort_adapter::AbortAdapter;
use crate::alloc::{
    Stage0Alloc as Alloc, Global, Layout, stage0_phantom, stage0_unphantom
};
use crate::vec::Vec;
use crate::raw_vec::RawVec;
use crate::str::from_boxed_utf8_unchecked;

/// A pointer type for heap allocation.
///
/// See the [module-level documentation](../../std/boxed/index.html) for more.
#[cfg(not(stage0))]
#[lang = "owned_box"]
#[fundamental]
#[stable(feature = "rust1", since = "1.0.0")]
pub struct Box<T: ?Sized, A = AbortAdapter<Global>>(Unique<T>, pub(crate) A);

// Use a variant with PhantomData in stage0, to satisfy the limitations of
// DispatchFromDyn in 1.35.
#[allow(missing_docs)]
#[cfg(stage0)]
#[lang = "owned_box"]
#[fundamental]
#[stable(feature = "rust1", since = "1.0.0")]
pub struct Box<T: ?Sized, A = AbortAdapter<Global>>(Unique<T>, pub(crate) PhantomData<A>);

impl<T> Box<T> {
    /// Allocates memory on the heap and then places `x` into it.
    ///
    /// This doesn't actually allocate if `T` is zero-sized.
    ///
    /// # Examples
    ///
    /// ```
    /// let five = Box::new(5);
    /// ```
    #[stable(feature = "rust1", since = "1.0.0")]
    #[inline(always)]
    pub fn new(x: T) -> Box<T> {
        box x
    }

    /// Constructs a new `Pin<Box<T>>`. If `T` does not implement `Unpin`, then
    /// `x` will be pinned in memory and unable to be moved.
    #[stable(feature = "pin", since = "1.33.0")]
    #[inline(always)]
    pub fn pin(x: T) -> Pin<Box<T>> {
        (box x).into()
    }
}

impl<T, A: Alloc> Box<T, A> {
    /// Allocates memory in the given allocator and then places `x` into it.
    ///
    /// This doesn't actually allocate if `T` is zero-sized.
    ///
    /// # Examples
    ///
    /// ```
    /// # #![feature(allocator_api)]
    /// use std::alloc::Global;
    /// let five = Box::new_in(5, Global);
    /// ```
    #[unstable(feature = "allocator_api", issue = "32838")]
    #[inline(always)]
    pub fn new_in(x: T, a: A) -> Result<Box<T, A>, A::Err> {
        let mut a = a;
        let layout = Layout::for_value(&x);
        let size = layout.size();
        let ptr = if size == 0 {
            Unique::empty()
        } else {
            unsafe {
                let ptr = a.alloc(layout)?;
                ptr.cast().into()
            }
        };
        // Move x into the location allocated above. This needs to happen
        // for any size so that x is not dropped in some cases.
        unsafe {
            ptr::write(ptr.as_ptr() as *mut T, x);
        }
        Ok(Box(ptr, stage0_phantom(a)))
    }

    /// Constructs a new `Pin<Box<T>>`. If `T` does not implement `Unpin`, then
    /// `x` will be pinned in memory and unable to be moved.
    #[unstable(feature = "allocator_api", issue = "32838")]
    #[inline(always)]
    pub fn pin_in(x: T, a: A) -> Result<Pin<Box<T, A>>, A::Err> {
        Box::new_in(x, a).map(Into::into)
    }
}

impl<T: ?Sized> Box<T> {
    /// Constructs a box from a raw pointer.
    ///
    /// After calling this function, the raw pointer is owned by the
    /// resulting `Box`. Specifically, the `Box` destructor will call
    /// the destructor of `T` and free the allocated memory. Since the
    /// way `Box` allocates and releases memory is unspecified, the
    /// only valid pointer to pass to this function is the one taken
    /// from another `Box` via the [`Box::into_raw`] function.
    ///
    /// This function is unsafe because improper use may lead to
    /// memory problems. For example, a double-free may occur if the
    /// function is called twice on the same raw pointer.
    ///
    /// [`Box::into_raw`]: struct.Box.html#method.into_raw
    ///
    /// # Examples
    ///
    /// ```
    /// let x = Box::new(5);
    /// let ptr = Box::into_raw(x);
    /// let x = unsafe { Box::from_raw(ptr) };
    /// ```
    #[stable(feature = "box_raw", since = "1.4.0")]
    #[inline]
    pub unsafe fn from_raw(raw: *mut T) -> Self {
        Box(Unique::new_unchecked(raw), stage0_phantom(AbortAdapter(Global)))
    }
}

impl<T: ?Sized, A> Box<T, A> {
    /// Constructs a box from a raw pointer in the given allocator.
    ///
    /// This is similar to the [`Box::from_raw`] function, but assumes
    /// the pointer was allocated with the given allocator.
    ///
    /// This function is unsafe because improper use may lead to
    /// memory problems. For example, specifying the wrong allocator
    /// may corrupt the allocator state.
    ///
    /// [`Box::from_raw`]: struct.Box.html#method.from_raw
    ///
    /// # Examples
    ///
    /// ```
    /// # #![feature(allocator_api)]
    /// use std::alloc::Global;
    /// let x = Box::new_in(5, Global);
    /// let ptr = Box::into_raw(x);
    /// let x = unsafe { Box::from_raw_in(ptr, Global) };
    /// ```
    #[unstable(feature = "allocator_api", issue = "32838")]
    #[inline]
    pub unsafe fn from_raw_in(raw: *mut T, a: A) -> Self {
        Box(Unique::new_unchecked(raw), stage0_phantom(a))
    }

    /// Maps a `Box<T, A>` to `Box<U, A>` by applying a function to the
    /// raw pointer.
    #[unstable(feature = "allocator_api", issue = "32838")]
    #[inline]
    pub unsafe fn map_raw<U: ?Sized, F: FnOnce(*mut T) -> *mut U>(b: Box<T, A>, f: F) -> Box<U, A> {
        let a = ptr::read(&b.1);
        Box::from_raw_in(f(Box::into_raw(b)), stage0_unphantom(a))
    }
}

impl<T: ?Sized, A> Box<T, A> {
    /// Consumes the `Box`, returning a wrapped raw pointer.
    ///
    /// The pointer will be properly aligned and non-null.
    ///
    /// After calling this function, the caller is responsible for the
    /// memory previously managed by the `Box`. In particular, the
    /// caller should properly destroy `T` and release the memory. The
    /// proper way to do so is to convert the raw pointer back into a
    /// `Box` with the [`Box::from_raw`] function.
    ///
    /// Note: this is an associated function, which means that you have
    /// to call it as `Box::into_raw(b)` instead of `b.into_raw()`. This
    /// is so that there is no conflict with a method on the inner type.
    ///
    /// [`Box::from_raw`]: struct.Box.html#method.from_raw
    ///
    /// # Examples
    ///
    /// ```
    /// let x = Box::new(5);
    /// let ptr = Box::into_raw(x);
    /// ```
    #[stable(feature = "box_raw", since = "1.4.0")]
    #[inline]
    pub fn into_raw(b: Self) -> *mut T {
        Box::into_raw_non_null(b).as_ptr()
    }

    /// Consumes the `Box`, returning the wrapped pointer as `NonNull<T>`.
    ///
    /// After calling this function, the caller is responsible for the
    /// memory previously managed by the `Box`. In particular, the
    /// caller should properly destroy `T` and release the memory. The
    /// proper way to do so is to convert the `NonNull<T>` pointer
    /// into a raw pointer and back into a `Box` with the [`Box::from_raw`]
    /// function.
    ///
    /// Note: this is an associated function, which means that you have
    /// to call it as `Box::into_raw_non_null(b)`
    /// instead of `b.into_raw_non_null()`. This
    /// is so that there is no conflict with a method on the inner type.
    ///
    /// [`Box::from_raw`]: struct.Box.html#method.from_raw
    ///
    /// # Examples
    ///
    /// ```
    /// #![feature(box_into_raw_non_null)]
    ///
    /// fn main() {
    ///     let x = Box::new(5);
    ///     let ptr = Box::into_raw_non_null(x);
    /// }
    /// ```
    #[unstable(feature = "box_into_raw_non_null", issue = "47336")]
    #[inline]
    pub fn into_raw_non_null(b: Box<T, A>) -> NonNull<T> {
        Box::into_unique(b).into()
    }

    #[unstable(feature = "ptr_internals", issue = "0", reason = "use into_raw_non_null instead")]
    #[inline]
    #[doc(hidden)]
    pub fn into_unique(mut b: Box<T, A>) -> Unique<T> {
        // Box is kind-of a library type, but recognized as a "unique pointer" by
        // Stacked Borrows.  This function here corresponds to "reborrowing to
        // a raw pointer", but there is no actual reborrow here -- so
        // without some care, the pointer we are returning here still carries
        // the `Uniq` tag.  We round-trip through a mutable reference to avoid that.
        let unique = unsafe { b.0.as_mut() as *mut T };
        mem::forget(b);
        unsafe { Unique::new_unchecked(unique) }
    }


    #[unstable(feature = "unique", reason = "needs an RFC to flesh out design",
               issue = "27730")]
    #[inline]
    pub fn into_both(mut b: Self) -> (Unique<T>, A) {
        let unique = b.0;
        let alloc = unsafe {
            let mut a = mem::uninitialized();
            mem::swap(&mut a, &mut b.1);
            a
        };
        mem::forget(b);
        (unique, alloc)
    }

    /// Consumes and leaks the `Box`, returning a mutable reference,
    /// `&'a mut T`. Note that the type `T` must outlive the chosen lifetime
    /// `'a`. If the type has only static references, or none at all, then this
    /// may be chosen to be `'static`.
    ///
    /// This function is mainly useful for data that lives for the remainder of
    /// the program's life. Dropping the returned reference will cause a memory
    /// leak. If this is not acceptable, the reference should first be wrapped
    /// with the [`Box::from_raw`] function producing a `Box`. This `Box` can
    /// then be dropped which will properly destroy `T` and release the
    /// allocated memory.
    ///
    /// Note: this is an associated function, which means that you have
    /// to call it as `Box::leak(b)` instead of `b.leak()`. This
    /// is so that there is no conflict with a method on the inner type.
    ///
    /// [`Box::from_raw`]: struct.Box.html#method.from_raw
    ///
    /// # Examples
    ///
    /// Simple usage:
    ///
    /// ```
    /// fn main() {
    ///     let x = Box::new(41);
    ///     let static_ref: &'static mut usize = Box::leak(x);
    ///     *static_ref += 1;
    ///     assert_eq!(*static_ref, 42);
    /// }
    /// ```
    ///
    /// Unsized data:
    ///
    /// ```
    /// fn main() {
    ///     let x = vec![1, 2, 3].into_boxed_slice();
    ///     let static_ref = Box::leak(x);
    ///     static_ref[0] = 4;
    ///     assert_eq!(*static_ref, [4, 2, 3]);
    /// }
    /// ```
    #[stable(feature = "box_leak", since = "1.26.0")]
    #[inline]
    pub fn leak<'a>(b: Self) -> &'a mut T
    where
        T: 'a // Technically not needed, but kept to be explicit.
    {
        unsafe { &mut *Box::into_raw(b) }
    }

    /// Converts a `Box<T>` into a `Pin<Box<T>>`
    ///
    /// This conversion does not allocate on the heap and happens in place.
    ///
    /// This is also available via [`From`].
    #[unstable(feature = "box_into_pin", issue = "0")]
    pub fn into_pin(boxed: Box<T, A>) -> Pin<Box<T, A>> {
        // It's not possible to move or replace the insides of a `Pin<Box<T>>`
        // when `T: !Unpin`,  so it's safe to pin it directly without any
        // additional requirements.
        unsafe { Pin::new_unchecked(boxed) }
    }
}


#[stable(feature = "rust1", since = "1.0.0")]
unsafe impl<#[may_dangle] T: ?Sized, A> Drop for Box<T, A> {
    fn drop(&mut self) {
        // FIXME: Do nothing, drop is currently performed by compiler.
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<T: Default, A: Alloc<Err = !> + Default> Default for Box<T, A> {
    /// Creates a `Box<T, A>`, with the `Default` value for T.
    fn default() -> Box<T, A> {
        let Ok(b) = Box::new_in(Default::default(), A::default());
        b
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<T, A: Alloc<Err=!> + Default> Default for Box<[T], A> {
    fn default() -> Box<[T], A> {
        let Ok(b) = Box::<[T; 0], A>::new_in([], Default::default());
        b
    }
}

#[stable(feature = "default_box_extra", since = "1.17.0")]
impl<A: Alloc<Err = !> + Default> Default for Box<str, A> {
    fn default() -> Box<str, A> {
        unsafe { from_boxed_utf8_unchecked(Default::default()) }
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<T: Clone, A: Alloc<Err = !> + Clone> Clone for Box<T, A> {
    /// Returns a new box with a `clone()` of this box's contents.
    ///
    /// # Examples
    ///
    /// ```
    /// let x = Box::new(5);
    /// let y = x.clone();
    /// ```
    #[rustfmt::skip]
    #[inline]
    fn clone(&self) -> Self {
        let Ok(b) = Box::new_in((**self).clone(), stage0_unphantom(self.1.clone()));
        b
    }
    /// Copies `source`'s contents into `self` without creating a new allocation.
    ///
    /// # Examples
    ///
    /// ```
    /// let x = Box::new(5);
    /// let mut y = Box::new(10);
    ///
    /// y.clone_from(&x);
    ///
    /// assert_eq!(*y, 5);
    /// ```
    #[inline]
    fn clone_from(&mut self, source: &Self) {
        (**self).clone_from(&(**source));
    }
}

#[stable(feature = "box_slice_clone", since = "1.3.0")]
impl<A: Alloc<Err = !> + Clone> Clone for Box<str, A> {
    fn clone(&self) -> Self {
        let len = self.len();
        let Ok(buf) = RawVec::with_capacity_in(len, stage0_unphantom(self.1.clone()));
        unsafe {
            ptr::copy_nonoverlapping(self.as_ptr(), buf.ptr(), len);
            from_boxed_utf8_unchecked(buf.into_box())
        }
    }
}

/// Just the contents are compared, the allocator is ignored
#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized + PartialEq, A> PartialEq for Box<T, A> {
    #[inline]
    fn eq(&self, other: &Box<T, A>) -> bool {
        PartialEq::eq(&**self, &**other)
    }
    #[inline]
    fn ne(&self, other: &Box<T, A>) -> bool {
        PartialEq::ne(&**self, &**other)
    }
}
/// Just the contents are compared, the allocator is ignored
#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized + PartialOrd, A> PartialOrd for Box<T, A> {
    #[inline]
    fn partial_cmp(&self, other: &Box<T, A>) -> Option<Ordering> {
        PartialOrd::partial_cmp(&**self, &**other)
    }
    #[inline]
    fn lt(&self, other: &Box<T, A>) -> bool {
        PartialOrd::lt(&**self, &**other)
    }
    #[inline]
    fn le(&self, other: &Box<T, A>) -> bool {
        PartialOrd::le(&**self, &**other)
    }
    #[inline]
    fn ge(&self, other: &Box<T, A>) -> bool {
        PartialOrd::ge(&**self, &**other)
    }
    #[inline]
    fn gt(&self, other: &Box<T, A>) -> bool {
        PartialOrd::gt(&**self, &**other)
    }
}
/// Just the contents are compared, the allocator is ignored
#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized + Ord, A> Ord for Box<T, A> {
    #[inline]
    fn cmp(&self, other: &Box<T, A>) -> Ordering {
        Ord::cmp(&**self, &**other)
    }
}
/// Just the contents are compared, the allocator is ignored
#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized + Eq, A> Eq for Box<T, A> {}

/// Just the contents are compared, the allocator is ignored
#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized + Hash, A> Hash for Box<T, A> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

/// Just the contents are compared, the allocator is ignored
#[stable(feature = "indirect_hasher_impl", since = "1.22.0")]
impl<T: ?Sized + Hasher, A> Hasher for Box<T, A> {
    fn finish(&self) -> u64 {
        (**self).finish()
    }
    fn write(&mut self, bytes: &[u8]) {
        (**self).write(bytes)
    }
    fn write_u8(&mut self, i: u8) {
        (**self).write_u8(i)
    }
    fn write_u16(&mut self, i: u16) {
        (**self).write_u16(i)
    }
    fn write_u32(&mut self, i: u32) {
        (**self).write_u32(i)
    }
    fn write_u64(&mut self, i: u64) {
        (**self).write_u64(i)
    }
    fn write_u128(&mut self, i: u128) {
        (**self).write_u128(i)
    }
    fn write_usize(&mut self, i: usize) {
        (**self).write_usize(i)
    }
    fn write_i8(&mut self, i: i8) {
        (**self).write_i8(i)
    }
    fn write_i16(&mut self, i: i16) {
        (**self).write_i16(i)
    }
    fn write_i32(&mut self, i: i32) {
        (**self).write_i32(i)
    }
    fn write_i64(&mut self, i: i64) {
        (**self).write_i64(i)
    }
    fn write_i128(&mut self, i: i128) {
        (**self).write_i128(i)
    }
    fn write_isize(&mut self, i: isize) {
        (**self).write_isize(i)
    }
}

#[stable(feature = "from_for_ptrs", since = "1.6.0")]
impl<T, A: Alloc<Err = !> + Default> From<T> for Box<T, A> {
    /// Converts a generic type `T` into a `Box<T>`
    ///
    /// The conversion allocates on the heap and moves `t`
    /// from the stack into it.
    ///
    /// # Examples
    /// ```rust
    /// let x = 5;
    /// let boxed = Box::new(5);
    ///
    /// assert_eq!(Box::from(x), boxed);
    /// ```
    fn from(t: T) -> Self {
        let Ok(b) = Box::new_in(t, Default::default());
        b
    }
}

#[stable(feature = "pin", since = "1.33.0")]
impl<T: ?Sized, A> From<Box<T, A>> for Pin<Box<T, A>> {
    /// Converts a `Box<T>` into a `Pin<Box<T>>`
    ///
    /// This conversion does not allocate on the heap and happens in place.
    fn from(boxed: Box<T, A>) -> Self {
        Box::into_pin(boxed)
    }
}

#[stable(feature = "box_from_slice", since = "1.17.0")]
impl<T: Copy, A: Alloc<Err = !> + Default> From<&[T]> for Box<[T], A> {
    /// Converts a `&[T]` into a `Box<[T]>`
    ///
    /// This conversion allocates on the heap
    /// and performs a copy of `slice`.
    ///
    /// # Examples
    /// ```rust
    /// // create a &[u8] which will be used to create a Box<[u8]>
    /// let slice: &[u8] = &[104, 101, 108, 108, 111];
    /// let boxed_slice: Box<[u8]> = Box::from(slice);
    ///
    /// println!("{:?}", boxed_slice);
    /// ```
    fn from(slice: &[T]) -> Box<[T], A> {
        let a = A::default();
        let Ok(vec) = RawVec::with_capacity_in(slice.len(), a);
        let mut boxed = unsafe { vec.into_box() };
        boxed.copy_from_slice(slice);
        boxed
    }
}

#[stable(feature = "box_from_slice", since = "1.17.0")]
impl<A: Alloc<Err = !> + Default> From<&str> for Box<str, A> {
    /// Converts a `&str` into a `Box<str>`
    ///
    /// This conversion allocates on the heap
    /// and performs a copy of `s`.
    ///
    /// # Examples
    /// ```rust
    /// let boxed: Box<str> = Box::from("hello");
    /// println!("{}", boxed);
    /// ```
    #[inline]
    fn from(s: &str) -> Box<str, A> {
        unsafe { from_boxed_utf8_unchecked(Box::from(s.as_bytes())) }
    }
}

#[stable(feature = "boxed_str_conv", since = "1.19.0")]
impl<A> From<Box<str, A>> for Box<[u8], A> {
    /// Converts a `Box<str>>` into a `Box<[u8]>`
    ///
    /// This conversion does not allocate on the heap and happens in place.
    ///
    /// # Examples
    /// ```rust
    /// // create a Box<str> which will be used to create a Box<[u8]>
    /// let boxed: Box<str> = Box::from("hello");
    /// let boxed_str: Box<[u8]> = Box::from(boxed);
    ///
    /// // create a &[u8] which will be used to create a Box<[u8]>
    /// let slice: &[u8] = &[104, 101, 108, 108, 111];
    /// let boxed_slice = Box::from(slice);
    ///
    /// assert_eq!(boxed_slice, boxed_str);
    /// ```
    #[inline]
    fn from(s: Box<str, A>) -> Self {
        unsafe { Box::map_raw(s, |p| p as *mut [u8]) }
    }
}

impl<A> Box<dyn Any, A> {
    #[inline]
    #[stable(feature = "rust1", since = "1.0.0")]
    /// Attempt to downcast the box to a concrete type.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::any::Any;
    ///
    /// fn print_if_string(value: Box<dyn Any>) {
    ///     if let Ok(string) = value.downcast::<String>() {
    ///         println!("String ({}): {}", string.len(), string);
    ///     }
    /// }
    ///
    /// fn main() {
    ///     let my_string = "Hello World".to_string();
    ///     print_if_string(Box::new(my_string));
    ///     print_if_string(Box::new(0i8));
    /// }
    /// ```
    pub fn downcast<T: Any>(self) -> Result<Box<T, A>, Box<dyn Any, A>> {
        if self.is::<T>() {
            unsafe { Ok(Box::map_raw(self, |p| p as *mut T)) }
        } else {
            Err(self)
        }
    }
}

impl<A: Alloc<Err=!>> Box<dyn Any + Send, A> {
    #[inline]
    #[stable(feature = "rust1", since = "1.0.0")]
    /// Attempt to downcast the box to a concrete type.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::any::Any;
    ///
    /// fn print_if_string(value: Box<dyn Any + Send>) {
    ///     if let Ok(string) = value.downcast::<String>() {
    ///         println!("String ({}): {}", string.len(), string);
    ///     }
    /// }
    ///
    /// fn main() {
    ///     let my_string = "Hello World".to_string();
    ///     print_if_string(Box::new(my_string));
    ///     print_if_string(Box::new(0i8));
    /// }
    /// ```
    pub fn downcast<T: Any>(self) -> Result<Box<T, A>, Box<dyn Any + Send, A>> {
        <Box<dyn Any, A>>::downcast(self).map_err(|s| unsafe {
            // reapply the Send marker
            Box::map_raw(s, |p| p as *mut (dyn Any + Send))
        })
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<T: fmt::Display + ?Sized, A> fmt::Display for Box<T, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, f)
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<T: fmt::Debug + ?Sized, A> fmt::Debug for Box<T, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized, A> fmt::Pointer for Box<T, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // It's not possible to extract the inner Uniq directly from the Box,
        // instead we cast it to a *const which aliases the Unique
        let ptr: *const T = &**self;
        fmt::Pointer::fmt(&ptr, f)
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized, A> Deref for Box<T, A> {
    type Target = T;

    fn deref(&self) -> &T {
        &**self
    }
}

#[stable(feature = "rust1", since = "1.0.0")]
impl<T: ?Sized, A> DerefMut for Box<T, A> {
    fn deref_mut(&mut self) -> &mut T {
        &mut **self
    }
}

#[unstable(feature = "receiver_trait", issue = "0")]
impl<T: ?Sized, A> Receiver for Box<T, A> {}

#[stable(feature = "rust1", since = "1.0.0")]
impl<I: Iterator + ?Sized, A> Iterator for Box<I, A> {
    type Item = I::Item;
    fn next(&mut self) -> Option<I::Item> {
        (**self).next()
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (**self).size_hint()
    }
    fn nth(&mut self, n: usize) -> Option<I::Item> {
        (**self).nth(n)
    }
}
#[stable(feature = "rust1", since = "1.0.0")]
impl<I: DoubleEndedIterator + ?Sized, A> DoubleEndedIterator for Box<I, A> {
    fn next_back(&mut self) -> Option<I::Item> {
        (**self).next_back()
    }
    fn nth_back(&mut self, n: usize) -> Option<I::Item> {
        (**self).nth_back(n)
    }
}
#[stable(feature = "rust1", since = "1.0.0")]
impl<I: ExactSizeIterator + ?Sized, A> ExactSizeIterator for Box<I, A> {
    fn len(&self) -> usize {
        (**self).len()
    }
    fn is_empty(&self) -> bool {
        (**self).is_empty()
    }
}

#[stable(feature = "fused", since = "1.26.0")]
impl<I: FusedIterator + ?Sized, A> FusedIterator for Box<I, A> {}

#[stable(feature = "boxed_closure_impls", since = "1.35.0")]
impl<A, F: FnOnce<A> + ?Sized, Alloc> FnOnce<A> for Box<F, Alloc> {
    type Output = <F as FnOnce<A>>::Output;

    extern "rust-call" fn call_once(self, args: A) -> Self::Output {
        <F as FnOnce<A>>::call_once(*self, args)
    }
}

#[stable(feature = "boxed_closure_impls", since = "1.35.0")]
impl<A, F: FnMut<A> + ?Sized, Alloc> FnMut<A> for Box<F, Alloc> {
    extern "rust-call" fn call_mut(&mut self, args: A) -> Self::Output {
        <F as FnMut<A>>::call_mut(self, args)
    }
}

#[stable(feature = "boxed_closure_impls", since = "1.35.0")]
impl<A, F: Fn<A> + ?Sized, Alloc> Fn<A> for Box<F, Alloc> {
    extern "rust-call" fn call(&self, args: A) -> Self::Output {
        <F as Fn<A>>::call(self, args)
    }
}

/// `FnBox` is a version of the `FnOnce` intended for use with boxed
/// closure objects. The idea is that where one would normally store a
/// `Box<dyn FnOnce()>` in a data structure, you should use
/// `Box<dyn FnBox()>`. The two traits behave essentially the same, except
/// that a `FnBox` closure can only be called if it is boxed. (Note
/// that `FnBox` may be deprecated in the future if `Box<dyn FnOnce()>`
/// closures become directly usable.)
///
/// # Examples
///
/// Here is a snippet of code which creates a hashmap full of boxed
/// once closures and then removes them one by one, calling each
/// closure as it is removed. Note that the type of the closures
/// stored in the map is `Box<dyn FnBox() -> i32>` and not `Box<dyn FnOnce()
/// -> i32>`.
///
/// ```
/// #![feature(fnbox)]
///
/// use std::boxed::FnBox;
/// use std::collections::HashMap;
///
/// fn make_map() -> HashMap<i32, Box<dyn FnBox() -> i32>> {
///     let mut map: HashMap<i32, Box<dyn FnBox() -> i32>> = HashMap::new();
///     map.insert(1, Box::new(|| 22));
///     map.insert(2, Box::new(|| 44));
///     map
/// }
///
/// fn main() {
///     let mut map = make_map();
///     for i in &[1, 2] {
///         let f = map.remove(&i).unwrap();
///         assert_eq!(f(), i * 22);
///     }
/// }
/// ```
#[rustc_paren_sugar]
#[unstable(feature = "fnbox",
           reason = "will be deprecated if and when `Box<FnOnce>` becomes usable", issue = "28796")]
pub trait FnBox<Args, A: Alloc = AbortAdapter<Global>>: FnOnce<Args> {
    /// Performs the call operation.
    fn call_box(self: Box<Self, A>, args: Args) -> Self::Output;
}

//FIXME: Make generic over A along with DispatchFromDyn.
#[unstable(feature = "fnbox",
           reason = "will be deprecated if and when `Box<FnOnce>` becomes usable", issue = "28796")]
impl<Args, F> FnBox<Args> for F
    where F: FnOnce<Args>
{
    fn call_box(self: Box<F>, args: Args) -> F::Output {
        self.call_once(args)
    }
}

#[unstable(feature = "coerce_unsized", issue = "27732")]
impl<T: ?Sized + Unsize<U>, U: ?Sized, A> CoerceUnsized<Box<U, A>> for Box<T, A> {}

//FIXME: Make generic over A when the compiler supports it.
#[unstable(feature = "dispatch_from_dyn", issue = "0")]
impl<T: ?Sized + Unsize<U>, U: ?Sized> DispatchFromDyn<Box<U>> for Box<T> {}

#[stable(feature = "boxed_slice_from_iter", since = "1.32.0")]
impl<A> FromIterator<A> for Box<[A]> {
    fn from_iter<T: IntoIterator<Item = A>>(iter: T) -> Self {
        iter.into_iter().collect::<Vec<_>>().into_boxed_slice()
    }
}

#[stable(feature = "box_slice_clone", since = "1.3.0")]
impl<T: Clone, A: Alloc<Err = !> + Clone> Clone for Box<[T], A> {
    fn clone(&self) -> Self {
        let Ok(b) = RawVec::with_capacity_in(self.len(), self.1.clone());

        let mut new = BoxBuilder {
            data: b,
            len: 0,
        };

        let mut target = new.data.ptr();

        for item in self.iter() {
            unsafe {
                ptr::write(target, item.clone());
                target = target.offset(1);
            };

            new.len += 1;
        }

        return unsafe { new.into_box() };

        // Helper type for responding to panics correctly.
        struct BoxBuilder<T, A: Alloc> {
            data: RawVec<T, A>,
            len: usize,
        }

        impl<T, A: Alloc> BoxBuilder<T, A> {
            unsafe fn into_box(self) -> Box<[T], A> {
                let raw = ptr::read(&self.data);
                mem::forget(self);
                raw.into_box()
            }
        }

        impl<T, A: Alloc> Drop for BoxBuilder<T, A> {
            fn drop(&mut self) {
                let mut data = self.data.ptr();
                let max = unsafe { data.add(self.len) };

                while data != max {
                    unsafe {
                        ptr::read(data);
                        data = data.offset(1);
                    }
                }
            }
        }
    }
}

#[stable(feature = "box_borrow", since = "1.1.0")]
impl<T: ?Sized, A> borrow::Borrow<T> for Box<T, A> {
    fn borrow(&self) -> &T {
        &**self
    }
}

#[stable(feature = "box_borrow", since = "1.1.0")]
impl<T: ?Sized, A> borrow::BorrowMut<T> for Box<T, A> {
    fn borrow_mut(&mut self) -> &mut T {
        &mut **self
    }
}

#[stable(since = "1.5.0", feature = "smart_ptr_as_ref")]
impl<T: ?Sized, A> AsRef<T> for Box<T, A> {
    fn as_ref(&self) -> &T {
        &**self
    }
}

#[stable(since = "1.5.0", feature = "smart_ptr_as_ref")]
impl<T: ?Sized, A> AsMut<T> for Box<T, A> {
    fn as_mut(&mut self) -> &mut T {
        &mut **self
    }
}

/* Nota bene
 *
 *  We could have chosen not to add this impl, and instead have written a
 *  function of Pin<Box<T>> to Pin<T>. Such a function would not be sound,
 *  because Box<T> implements Unpin even when T does not, as a result of
 *  this impl.
 *
 *  We chose this API instead of the alternative for a few reasons:
 *      - Logically, it is helpful to understand pinning in regard to the
 *        memory region being pointed to. For this reason none of the
 *        standard library pointer types support projecting through a pin
 *        (Box<T> is the only pointer type in std for which this would be
 *        safe.)
 *      - It is in practice very useful to have Box<T> be unconditionally
 *        Unpin because of trait objects, for which the structural auto
 *        trait functionality does not apply (e.g., Box<dyn Foo> would
 *        otherwise not be Unpin).
 *
 *  Another type with the same semantics as Box but only a conditional
 *  implementation of `Unpin` (where `T: Unpin`) would be valid/safe, and
 *  could have a method to project a Pin<T> from it.
 */
#[stable(feature = "pin", since = "1.33.0")]
impl<T: ?Sized, A> Unpin for Box<T, A> { }

#[unstable(feature = "generator_trait", issue = "43122")]
impl<G: ?Sized + Generator + Unpin, A> Generator for Box<G, A> {
    type Yield = G::Yield;
    type Return = G::Return;

    fn resume(mut self: Pin<&mut Self>) -> GeneratorState<Self::Yield, Self::Return> {
        G::resume(Pin::new(&mut *self))
    }
}

#[unstable(feature = "generator_trait", issue = "43122")]
impl<G: ?Sized + Generator, A> Generator for Pin<Box<G, A>> {
    type Yield = G::Yield;
    type Return = G::Return;

    fn resume(mut self: Pin<&mut Self>) -> GeneratorState<Self::Yield, Self::Return> {
        G::resume((*self).as_mut())
    }
}

#[stable(feature = "futures_api", since = "1.36.0")]
impl<F: ?Sized + Future + Unpin, A> Future for Box<F, A> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        F::poll(Pin::new(&mut *self), cx)
    }
}
