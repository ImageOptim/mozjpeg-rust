pub trait VecUninitExtender {
    unsafe fn extend_uninit(&mut self, items: usize);
}

impl<T: Copy> VecUninitExtender for Vec<T> {
    unsafe fn extend_uninit(&mut self, items: usize) {
        let new_len = self.len() + items;
        self.try_reserve_exact(items).expect("oom");
        debug_assert!(self.capacity() >= new_len);
        self.set_len(new_len);
    }
}
