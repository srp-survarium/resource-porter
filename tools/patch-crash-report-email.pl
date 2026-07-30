#!/usr/bin/env perl
use strict;
use warnings;

use Digest::SHA qw(sha256_hex);
use File::Basename qw(basename dirname);
use File::Copy qw(copy);
use File::Temp qw(tempfile);
use Getopt::Long qw(GetOptions);

my $BUILD_802_SIZE = 12_235_264;
my $RECIPIENT_OFFSET = 0x943af8;
my $ORIGINAL_RECIPIENT = 'game_crash_reports@survarium.com';
my $INERT_RECIPIENT = 'crash-reports@localhost.invalid';
my $REPLACEMENT = $INERT_RECIPIENT . "\0";
my $ORIGINAL_SHA256 =
    '169d42c0fc25bddc285f5e93e26a2769edbdbf3d50b9e0575d9eed4757b74b0b';
my $PATCHED_SHA256 =
    'f7459d65611d20fcafc8b1db5e3fa3aae1013c22238c779787a79e70e4fd50f7';

sub read_binary {
    my ($path) = @_;
    open my $input, '<:raw', $path or die "cannot read $path: $!\n";
    local $/;
    my $data = <$input>;
    close $input or die "cannot close $path: $!\n";
    return $data;
}

sub literal_count {
    my ($data, $needle) = @_;
    my ($count, $offset) = (0, 0);
    while (($offset = index($data, $needle, $offset)) >= 0) {
        ++$count;
        $offset += length($needle);
    }
    return $count;
}

sub validate_common {
    my ($target, $image) = @_;
    die "fixed-size recipient patch has changed length\n"
        if length($ORIGINAL_RECIPIENT) != length($REPLACEMENT);
    die "$target: expected build 802 size $BUILD_802_SIZE, got "
        . length($image) . "\n"
        if length($image) != $BUILD_802_SIZE;
}

sub validate_patched {
    my ($target, $image) = @_;
    my $current = substr($image, $RECIPIENT_OFFSET, length($REPLACEMENT));
    die sprintf("%s: inert recipient is not present at 0x%x\n",
                $target, $RECIPIENT_OFFSET)
        if $current ne $REPLACEMENT;
    die "$target: original developer recipient is still present\n"
        if index($image, $ORIGINAL_RECIPIENT) >= 0;
    die "$target: inert recipient is not unique\n"
        if literal_count($image, $INERT_RECIPIENT) != 1;
}

sub verified_message {
    my ($target, $image) = @_;
    return "crash email patch verified: $target sha256="
        . sha256_hex($image)
        . " recipient=$INERT_RECIPIENT\n";
}

my ($target, $backup, $check);
GetOptions(
    'target=s' => \$target,
    'backup=s' => \$backup,
    'check'    => \$check,
) or die "usage: $0 --target FILE [--backup FILE] [--check]\n";
die "usage: $0 --target FILE [--backup FILE] [--check]\n"
    if !defined($target) || $target eq '';
die "missing build 802 executable: $target\n" if !-f $target;
$backup //= "$target.original-crash-email.bak";

my $image = read_binary($target);
validate_common($target, $image);

if ($check) {
    validate_patched($target, $image);
    print verified_message($target, $image);
    exit 0;
}

my $current =
    substr($image, $RECIPIENT_OFFSET, length($ORIGINAL_RECIPIENT));
if ($current eq $REPLACEMENT) {
    validate_patched($target, $image);
    print verified_message($target, $image);
    exit 0;
}
die sprintf("%s: unexpected bytes at recipient offset 0x%x\n",
            $target, $RECIPIENT_OFFSET)
    if $current ne $ORIGINAL_RECIPIENT;

my $digest = sha256_hex($image);
die "$target: recipient is original, but build hash is unsupported: $digest\n"
    if $digest ne $ORIGINAL_SHA256;
die "$target: original recipient is not unique\n"
    if literal_count($image, $ORIGINAL_RECIPIENT) != 1;

my $mode = (stat($target))[2] & 07777;
if (-e $backup) {
    my $backup_image = read_binary($backup);
    die "$backup: existing backup is not the stock build 802 image\n"
        if sha256_hex($backup_image) ne $ORIGINAL_SHA256;
} else {
    copy($target, $backup) or die "cannot create $backup: $!\n";
    chmod $mode, $backup or die "cannot set mode on $backup: $!\n";
}

substr($image, $RECIPIENT_OFFSET, length($ORIGINAL_RECIPIENT), $REPLACEMENT);
validate_patched($target, $image);
my $updated_digest = sha256_hex($image);
die "internal error: patched image hash is $updated_digest, "
    . "expected $PATCHED_SHA256\n"
    if $updated_digest ne $PATCHED_SHA256;

my ($output, $temporary) = tempfile(
    '.' . basename($target) . '.XXXXXX',
    DIR    => dirname($target),
    UNLINK => 0,
);
binmode $output;
print {$output} $image or die "cannot write $temporary: $!\n";
close $output or die "cannot close $temporary: $!\n";
chmod $mode, $temporary or die "cannot set mode on $temporary: $!\n";
rename $temporary, $target or die "cannot replace $target: $!\n";

print sprintf(
    "patched build 802 crash recipient at 0x%x: %s; backup=%s; sha256=%s\n",
    $RECIPIENT_OFFSET,
    $INERT_RECIPIENT,
    $backup,
    $updated_digest,
);
