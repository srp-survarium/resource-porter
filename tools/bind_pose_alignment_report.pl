#!/usr/bin/env perl
use strict;
use warnings;
use JSON::PP qw(decode_json);

sub read_file {
    my ($path) = @_;
    open my $fh, '<:raw', $path or die "cannot open $path: $!\n";
    local $/;
    return <$fh>;
}

sub parse_bind_pose {
    my ($path) = @_;
    my $bytes = read_file($path);
    my $count = unpack 'v', substr($bytes, 0, 2);
    my $offset = 2;
    my %bones;
    for my $index (0 .. $count - 1) {
        my $end = index($bytes, "\0", $offset);
        die "unterminated bone $index in $path\n" if $end < 0;
        my $name = substr($bytes, $offset, $end - $offset);
        $offset = $end + 1;
        my @frame = unpack 'f<9', substr($bytes, $offset, 36);
        $offset += 36;
        $bones{$name} = {
            translation => [@frame[0 .. 2]],
            matrix => frame_matrix(@frame),
        };
    }
    return \%bones;
}

sub multiply {
    my ($left, $right) = @_;
    my @result;
    for my $row (0 .. 3) {
        for my $column (0 .. 3) {
            $result[$row][$column] = 0.0;
            for my $inner (0 .. 3) {
                $result[$row][$column] +=
                    $left->[$row][$inner] * $right->[$inner][$column];
            }
        }
    }
    return \@result;
}

sub frame_matrix {
    my ($tx, $ty, $tz, $x, $y, $z, $sx, $sy, $sz) = @_;
    my ($xs, $xc) = (sin($x), cos($x));
    my ($ys, $yc) = (sin($y), cos($y));
    my ($zs, $zc) = (sin($z), cos($z));
    my $scale = [
        [$sx, 0, 0, 0], [0, $sy, 0, 0],
        [0, 0, $sz, 0], [0, 0, 0, 1],
    ];
    my $rotation = [
        [$yc * $zc, -$yc * $zs, $ys, 0],
        [$ys * $xs * $zc + $xc * $zs,
         -$xs * $ys * $zs + $xc * $zc, -$xs * $yc, 0],
        [-$xc * $ys * $zc + $xs * $zs,
         $xc * $ys * $zs + $xs * $zc, $xc * $yc, 0],
        [0, 0, 0, 1],
    ];
    my $translation = [
        [1, 0, 0, 0], [0, 1, 0, 0],
        [0, 0, 1, 0], [$tx, $ty, $tz, 1],
    ];
    return multiply(multiply($scale, $rotation), $translation);
}

sub inverse_affine {
    my ($m) = @_;
    my $det =
        $m->[0][0] * ($m->[1][1] * $m->[2][2] - $m->[1][2] * $m->[2][1])
        - $m->[0][1] * ($m->[1][0] * $m->[2][2] - $m->[1][2] * $m->[2][0])
        + $m->[0][2] * ($m->[1][0] * $m->[2][1] - $m->[1][1] * $m->[2][0]);
    die "singular bind frame\n" if abs($det) < 1e-7;
    my $r = 1.0 / $det;
    my @b = (
        [($m->[1][1] * $m->[2][2] - $m->[1][2] * $m->[2][1]) * $r,
         ($m->[0][2] * $m->[2][1] - $m->[0][1] * $m->[2][2]) * $r,
         ($m->[0][1] * $m->[1][2] - $m->[0][2] * $m->[1][1]) * $r],
        [($m->[1][2] * $m->[2][0] - $m->[1][0] * $m->[2][2]) * $r,
         ($m->[0][0] * $m->[2][2] - $m->[0][2] * $m->[2][0]) * $r,
         ($m->[0][2] * $m->[1][0] - $m->[0][0] * $m->[1][2]) * $r],
        [($m->[1][0] * $m->[2][1] - $m->[1][1] * $m->[2][0]) * $r,
         ($m->[0][1] * $m->[2][0] - $m->[0][0] * $m->[2][1]) * $r,
         ($m->[0][0] * $m->[1][1] - $m->[0][1] * $m->[1][0]) * $r],
    );
    my @t = @{$m->[3]}[0 .. 2];
    my @it = map {
        -($t[0] * $b[0][$_] + $t[1] * $b[1][$_] + $t[2] * $b[2][$_])
    } 0 .. 2;
    return [
        [@{$b[0]}, 0], [@{$b[1]}, 0], [@{$b[2]}, 0], [@it, 1],
    ];
}

sub transform {
    my ($point, $matrix) = @_;
    return [
        $point->[0] * $matrix->[0][0] + $point->[1] * $matrix->[1][0]
            + $point->[2] * $matrix->[2][0] + $matrix->[3][0],
        $point->[0] * $matrix->[0][1] + $point->[1] * $matrix->[1][1]
            + $point->[2] * $matrix->[2][1] + $matrix->[3][1],
        $point->[0] * $matrix->[0][2] + $point->[1] * $matrix->[1][2]
            + $point->[2] * $matrix->[2][2] + $matrix->[3][2],
    ];
}

my ($source_path, $target_path, $manifest_path) = @ARGV;
die "usage: $0 SOURCE_BIND_POSE TARGET_BIND_POSE MANIFEST\n" unless @ARGV == 3;
my $source = parse_bind_pose($source_path);
my $target = parse_bind_pose($target_path);
my $manifest = decode_json(read_file($manifest_path));
my $map = $manifest->{model}{bone_map};

for my $chain (
    [qw(LeftArm LeftForeArm LeftHand LeftHandMiddle_1 LeftHandMiddle_2 LeftHandMiddle_3)],
    [qw(RightArm RightForeArm RightHand RightHandMiddle_1 RightHandMiddle_2 RightHandMiddle_3)],
) {
    for my $index (0 .. $#$chain - 1) {
        my ($source_bone, $source_landmark) = @{$chain}[$index, $index + 1];
        my $target_bone = $map->{$source_bone};
        my $target_landmark = $map->{$source_landmark};
        my $delta = multiply(
            inverse_affine($source->{$source_bone}{matrix}),
            $target->{$target_bone}{matrix},
        );
        my $mapped = transform($source->{$source_landmark}{translation}, $delta);
        my $expected = $target->{$target_landmark}{translation};
        my $error = sqrt(
            ($mapped->[0] - $expected->[0]) ** 2
            + ($mapped->[1] - $expected->[1]) ** 2
            + ($mapped->[2] - $expected->[2]) ** 2
        );
        printf "%-20s -> %-20s landmark=%-20s error=%.5f"
            . " mapped=(%.5f,%.5f,%.5f) expected=(%.5f,%.5f,%.5f)\n",
            $source_bone, $target_bone, $target_landmark, $error,
            @$mapped, @$expected;
    }
}
