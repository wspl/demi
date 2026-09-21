# Cloud image pipeline

The selected pipeline produces an architecture-specific Linux root archive and
manifest for gVisor/systrap. Build inputs, artifact format, import, and acceptance
are defined in [Cloud images](../../docs/cloud-images.md). Deployment is described
in [Cloud setup](../../docs/managed-hosts-setup.md).

Implementation is pending. The kernel and ext4-base scripts currently in this
directory belong to the replaced implementation and are to be removed or rewritten;
they do not produce the selected release format.
