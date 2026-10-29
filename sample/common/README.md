# Shared sample assets

This directory stores files used by multiple sample projects.

The project-specific entries under [sample](../) point to this directory via symlinks so the same target definition or console interface is shared without repeating the same content in multiple places.

This is intended for the Git working tree on Unix-like systems. If a Windows environment does not allow symlinks, recreate the files explicitly in each project instead of relying on the shared path.
