//! The encoder owns its pass count. Vulkan submissions rotate only at complete
//! 2D chunk boundaries; other backends retain their submission behavior.

#[derive(Debug)]
pub struct CommandEncoder {
    inner: wgpu::CommandEncoder,
    render_passes: u64,
    chunk_submissions: u64,
    chunk_passes: u64,
    chunk_max_passes: u64,
}

impl From<wgpu::CommandEncoder> for CommandEncoder {
    fn from(inner: wgpu::CommandEncoder) -> Self {
        Self {
            inner,
            render_passes: 0,
            chunk_submissions: 0,
            chunk_passes: 0,
            chunk_max_passes: 0,
        }
    }
}

impl CommandEncoder {
    pub fn take_chunk_submission_stats(&mut self) -> (u64, u64, u64) {
        (
            std::mem::take(&mut self.chunk_submissions),
            std::mem::take(&mut self.chunk_passes),
            std::mem::take(&mut self.chunk_max_passes),
        )
    }

    // Submit at a completed chunk boundary, where no RenderPass borrows remain.
    // Profiling/debug scopes require the same encoder, so those builds retain
    // the original submission path, with no experimental rotation.
    #[cfg(not(any(feature = "profile-with-tracy", feature = "render_debug_labels")))]
    pub fn maybe_submit_chunk(
        &mut self,
        descriptors: &crate::Descriptors,
        staging_belt: &mut wgpu::util::StagingBelt,
    ) {
        // Driver-retention measurements currently cover Vulkan only.
        if descriptors.backend != wgpu::Backend::Vulkan {
            return;
        }
        static BUDGET: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
        let budget =
            *BUDGET.get_or_init(|| match std::env::var("RUFFLE_RENDER_CHUNK_PASS_BUDGET") {
                Ok(value) => effective_chunk_budget(descriptors.backend, Some(&value)),
                Err(std::env::VarError::NotPresent) => {
                    effective_chunk_budget(descriptors.backend, None)
                }
                Err(std::env::VarError::NotUnicode(_)) => {
                    tracing::warn!("Invalid render chunk pass budget; disabling chunk submissions");
                    0
                }
            });
        if budget == 0 || self.render_passes < budget {
            return;
        }
        staging_belt.finish();
        let old = std::mem::replace(
            &mut self.inner,
            descriptors
                .device
                .create_command_encoder(&Default::default()),
        );
        descriptors.queue.submit(Some(old.finish()));
        self.chunk_submissions += 1;
        self.chunk_passes += self.render_passes;
        self.chunk_max_passes = self.chunk_max_passes.max(self.render_passes);
        self.render_passes = 0;
        staging_belt.recall();
    }

    pub fn render_passes(&self) -> u64 {
        self.render_passes
    }

    pub fn begin_render_pass<'encoder>(
        &'encoder mut self,
        descriptor: &wgpu::RenderPassDescriptor<'_>,
    ) -> wgpu::RenderPass<'encoder> {
        self.render_passes += 1;
        self.inner.begin_render_pass(descriptor)
    }

    pub fn finish(self) -> wgpu::CommandBuffer {
        self.inner.finish()
    }
}

#[cfg(not(any(feature = "profile-with-tracy", feature = "render_debug_labels")))]
fn parse_chunk_budget(value: &str) -> Option<u64> {
    match value.parse::<u64>().ok()? {
        budget @ (0 | 32 | 128) => Some(budget),
        _ => None,
    }
}

#[cfg(not(any(feature = "profile-with-tracy", feature = "render_debug_labels")))]
fn effective_chunk_budget(backend: wgpu::Backend, value: Option<&str>) -> u64 {
    if backend != wgpu::Backend::Vulkan {
        return 0;
    }
    match value {
        None => 128,
        Some(value) => parse_chunk_budget(value).unwrap_or_else(|| {
            tracing::warn!("Invalid render chunk pass budget; disabling chunk submissions");
            0
        }),
    }
}

#[cfg(all(
    test,
    not(any(feature = "profile-with-tracy", feature = "render_debug_labels"))
))]
mod tests {
    use super::{effective_chunk_budget, parse_chunk_budget};

    #[test]
    fn non_vulkan_backends_ignore_even_an_explicit_budget() {
        for backend in [wgpu::Backend::Gl, wgpu::Backend::Dx12, wgpu::Backend::Metal] {
            assert_eq!(effective_chunk_budget(backend, None), 0);
            assert_eq!(effective_chunk_budget(backend, Some("128")), 0);
        }
    }

    #[test]
    fn vulkan_ordinary_launch_uses_budget_and_keeps_opt_out() {
        assert_eq!(effective_chunk_budget(wgpu::Backend::Vulkan, None), 128);
        assert_eq!(effective_chunk_budget(wgpu::Backend::Vulkan, Some("0")), 0);
        assert_eq!(
            effective_chunk_budget(wgpu::Backend::Vulkan, Some("128")),
            128
        );
        assert_eq!(effective_chunk_budget(wgpu::Backend::Vulkan, Some("64")), 0);
    }

    #[test]
    fn chunk_budget_rejects_invalid_configuration_without_panicking() {
        for budget in [0, 32, 128] {
            assert_eq!(parse_chunk_budget(&budget.to_string()), Some(budget));
        }
        for invalid in ["", "128passes", "-32", "64", "18446744073709551616"] {
            assert_eq!(parse_chunk_budget(invalid), None);
        }
    }
}

impl std::ops::Deref for CommandEncoder {
    type Target = wgpu::CommandEncoder;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for CommandEncoder {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl wgpu_profiler::ProfilerCommandRecorder for CommandEncoder {
    fn is_pass(&self) -> bool {
        false
    }
    fn write_timestamp(&mut self, query_set: &wgpu::QuerySet, query_index: u32) {
        self.inner.write_timestamp(query_set, query_index);
    }
    fn push_debug_group(&mut self, label: &str) {
        self.inner.push_debug_group(label);
    }
    fn pop_debug_group(&mut self) {
        self.inner.pop_debug_group();
    }
}

// wgpu-profiler's convenience method is specialized for its raw encoder. Keep
// the identical public query/parent/timestamp protocol for our owned wrapper.

pub(crate) trait ScopedRenderPass {
    fn scoped_render_pass(
        &mut self,
        label: impl Into<String>,
        descriptor: wgpu::RenderPassDescriptor<'_>,
    ) -> wgpu_profiler::OwningScope<'_, wgpu::RenderPass<'_>>;
}

impl ScopedRenderPass for wgpu_profiler::Scope<'_, CommandEncoder> {
    fn scoped_render_pass(
        &mut self,
        label: impl Into<String>,
        descriptor: wgpu::RenderPassDescriptor<'_>,
    ) -> wgpu_profiler::OwningScope<'_, wgpu::RenderPass<'_>> {
        let query = self
            .profiler
            .begin_pass_query(label, self.recorder)
            .with_parent(self.scope.as_ref());
        let pass = self
            .recorder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                timestamp_writes: query.render_pass_timestamp_writes(),
                label: descriptor.label.or(Some(&query.label)),
                ..descriptor
            });
        wgpu_profiler::OwningScope {
            profiler: self.profiler,
            recorder: pass,
            scope: Some(query),
        }
    }
}
