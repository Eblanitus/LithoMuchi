// Horizon 66f3981, MIT: verbatim Noise contract from noise/stack.rs.
pub trait Noise: std::fmt::Debug + Send + Sync {
    fn range(&self) -> Result<[f32; 2], NanRange>;
    fn get2(&self, x: f64, z: f64) -> f32;
    fn get3(&self, x: f64, y: f64, z: f64) -> f32;
    fn add_to_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        if buffer.len() < volume.size() {
            return Err(DensityError::BufferTooShort);
        }
        let [sx, sy, sz] = volume.sizes();
        let mut index = 0;
        for z in 0..sz {
            let z = f64::from(volume.coordinate(2, z)) * xz_scale;
            for x in 0..sx {
                let x = f64::from(volume.coordinate(0, x)) * xz_scale;
                for y in 0..sy {
                    let y = f64::from(volume.coordinate(1, y)) * y_scale;
                    buffer[index] += amplitude * self.get3(x, y, z);
                    index += 1;
                }
            }
        }
        Ok(())
    }
}
