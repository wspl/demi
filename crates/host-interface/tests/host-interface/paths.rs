//! A path a shell names, read against its directory as the Host reads it.
// Cost: none.

use demi_host_interface::HostPaths;

/// Each form a shell's path takes keeps what it means, `..` included.
#[test]
fn a_path_beside_a_directory_keeps_its_root_and_drive_and_its_dots() {
    let windows = |path| HostPaths::Windows.beside(r"C:\work\app", path);
    assert_eq!(windows(r".demi\workflows\review.yaml"), r"C:\work\app\.demi\workflows\review.yaml");
    assert_eq!(windows(r"deep\..\review.yaml"), r"C:\work\app\deep\..\review.yaml");
    assert_eq!(windows(r"\rules\review.yaml"), r"C:\rules\review.yaml");
    assert_eq!(windows("C:foo.yaml"), "C:foo.yaml");
    assert_eq!(windows(r"D:\rules\review.yaml"), r"D:\rules\review.yaml");
    assert_eq!(windows(r"\\server\share\review.yaml"), r"\\server\share\review.yaml");
    assert_eq!(windows("deep/review.yaml"), r"C:\work\app\deep\review.yaml");
    let posix = |path| HostPaths::Posix.beside("/work/app", path);
    assert_eq!(posix("deep/../review.yaml"), "/work/app/deep/../review.yaml");
    assert_eq!(posix("/rules/review.yaml"), "/rules/review.yaml");
}
