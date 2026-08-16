#!/usr/bin/env perl
#
# Remove build 802's five-second login-menu status wait.
#
# The login menu holds each status line for a fixed time before it will move
# on. `CLoginMenu::set_status` computes the deadline as *now plus a literal*:
#
#     mov  edx,[esi+0A8h]        ; the menu's clock
#     mov  eax,[edx+3F4h]        ; current time in ms
#     add  eax,1388h             ; + 5000
#     mov  [esi+0C0h],eax        ; -> deadline
#
# Four of those branches add 5000; one adds 10000. Replacing the four `add`
# instructions with nops leaves the deadline equal to the current time, so each
# status clears on the next frame and the menu proceeds as fast as the servers
# answer. Nothing else moves: the instructions are replaced in place, byte for
# byte, so every address in the image stays where it was.
#
# The 10000ms branch is left alone. It is not a login step -- and it is what
# tells this patcher it is looking at the right function, since it sits between
# the third and fourth sites.
#
# Transcribed from `srp/resources/patches/login_menu_set_status_patch.asm`,
# which records `rip = 0x6f1e77` for the first site. This build's file offsets
# agree with those addresses exactly.
#
# Idempotent, and refuses anything it does not recognise: every site must be
# found the expected number of times, either patched or unpatched but not a mix.
#
# Independently confirmed: `patch-crash-report-email.pl` records two accepted
# build 802 images whose only difference was never explained. Running this patch
# over the crash-patched second image
# (db61aaf319d25f946e3d58ab4a042e548462b627188d5a623d73fc9d9407adfe) produces
# f7459d65611d20fcafc8b1db5e3fa3aae1013c22238c779787a79e70e4fd50f7 -- exactly
# the crash-patched *first* image. The difference between them was this patch,
# applied by hand before it was written down.

use strict;
use warnings;

use Digest::SHA qw(sha256_hex);
use File::Basename qw(basename dirname);
use File::Copy qw(copy);
use File::Temp qw(tempfile);
use Getopt::Long qw(GetOptions);

my $BUILD_802_SIZE = 12_235_264;

# Each site is the `add` to remove, plus the `mov [esi+0C0h],reg` that follows
# it. The successor is what makes the pattern unique: `add eax,1388h` alone
# appears elsewhere in a 12MB image, but not written into that field.
#
# `count` because the function reaches the same instruction from two branches:
# `add eax,1388h` is written twice, once before the 10000ms branch and once
# after.
my @SITES = (
    {
        name    => 'add ecx,1388h',
        add     => "\x81\xC1\x88\x13\x00\x00",
        follows => "\x89\x8E\xC0\x00\x00\x00",
        count   => 1,
    },
    {
        name    => 'add eax,1388h',
        add     => "\x05\x88\x13\x00\x00",
        follows => "\x89\x86\xC0\x00\x00\x00",
        count   => 2,
    },
    {
        name    => 'add edx,1388h',
        add     => "\x81\xC2\x88\x13\x00\x00",
        follows => "\x89\x96\xC0\x00\x00\x00",
        count   => 1,
    },
);

# The one wait this patch does not touch, and the landmark that says the four
# above are the login menu's rather than four coincidences.
my $KEEP = {
    name    => 'add ecx,2710h',
    add     => "\x81\xC1\x10\x27\x00\x00",
    follows => "\x89\x8E\xC0\x00\x00\x00",
    count   => 1,
};

sub read_binary {
    my ($path) = @_;
    open my $input, '<:raw', $path or die "cannot read $path: $!\n";
    local $/;
    my $data = <$input>;
    close $input or die "cannot close $path: $!\n";
    return $data;
}

sub offsets_of {
    my ($image, $needle) = @_;
    my ($offset, @found) = (0);
    while (($offset = index($image, $needle, $offset)) >= 0) {
        push @found, $offset;
        $offset += 1;
    }
    return @found;
}

# Where each site sits, and whether it still carries its `add`.
#
# A site is either the original instruction or the nops that replaced it. Both
# are accepted; anything else, or any count other than one, is a refusal.
sub locate {
    my ($target, $image) = @_;
    # The landmark first, and by its original bytes only. Once the `add
    # ecx,1388h` site is nopped its bytes are indistinguishable from a nopped
    # `add ecx,2710h` -- both are six nops before `mov [esi+0C0h],ecx` -- so
    # searching for a removed landmark would find the site this patch just
    # removed. Its presence is the whole check: if the 10000ms wait is gone,
    # this is not the image this patch describes.
    my @landmark = offsets_of($image, $KEEP->{add} . $KEEP->{follows});
    die "$target: $KEEP->{name} appears "
        . scalar(@landmark)
        . " times, expected $KEEP->{count}; this is not build 802's login menu\n"
        if @landmark != $KEEP->{count};

    my (@sites, $patched);
    for my $site (@SITES) {
        my $original = $site->{add} . $site->{follows};
        my $replaced = ("\x90" x length($site->{add})) . $site->{follows};
        my @original = offsets_of($image, $original);
        my @replaced = offsets_of($image, $replaced);

        die "$target: $site->{name} appears "
            . (@original + @replaced)
            . " times, expected $site->{count}\n"
            if @original + @replaced != $site->{count};

        # All occurrences of one instruction move together, so a site that is
        # part patched and part not is a corrupt image rather than a state to
        # repair.
        die "$target: the login waits are half patched; "
            . "restore the backup and start again\n"
            if @original && @replaced;
        my $is_patched = @replaced ? 1 : 0;
        die "$target: the login waits are half patched; "
            . "restore the backup and start again\n"
            if defined($patched) && $patched != $is_patched;
        $patched = $is_patched;
        push @sites, map { { %$site, offset => $_ } }
            ($is_patched ? @replaced : @original);
    }

    @sites = sort { $a->{offset} <=> $b->{offset} } @sites;
    die "$target: expected four login waits, found " . scalar(@sites) . "\n"
        if @sites != 4;
    return ($patched, @sites);
}

sub verified_message {
    my ($target, $image, @sites) = @_;
    return "login wait patch verified: $target sha256="
        . sha256_hex($image)
        . ' sites='
        . join(',', map { sprintf('0x%x', $_->{offset}) } @sites) . "\n";
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
$backup //= "$target.original-login-wait.bak";

my $image = read_binary($target);
die "$target: expected build 802 size $BUILD_802_SIZE, got "
    . length($image) . "\n"
    if length($image) != $BUILD_802_SIZE;

my ($patched, @sites) = locate($target, $image);

if ($patched) {
    print verified_message($target, $image, @sites);
    exit 0;
}
die "$target: the login waits are still in place\n" if $check;

my $mode = (stat($target))[2] & 07777;
if (!-e $backup) {
    copy($target, $backup) or die "cannot create $backup: $!\n";
    chmod $mode, $backup or die "cannot set mode on $backup: $!\n";
}

for my $site (@sites) {
    substr($image, $site->{offset}, length($site->{add}),
        "\x90" x length($site->{add}));
}

# Re-read the image the way a fresh run would, so the patch is confirmed by the
# same code that would refuse it.
my ($confirmed) = locate($target, $image);
die "internal error: the patched image does not read back as patched\n"
    if !$confirmed;

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

print verified_message($target, $image, @sites);
