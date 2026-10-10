#!/usr/bin/env perl
# SPDX-License-Identifier: GPL-3.0-or-later
use strict;
use warnings;

@ARGV == 2 or die "usage: $0 VERTICES INDICES\n";

sub read_file {
    my ($path) = @_;
    open my $file, '<:raw', $path or die "cannot open $path: $!\n";
    local $/;
    return <$file>;
}

my ($vertex_path, $index_path) = @ARGV;
my $vertex_file = read_file($vertex_path);
my $index_file = read_file($index_path);

unpack('L<', substr($vertex_file, 0, 4)) == 0x19
    or die "vertices do not start with chunk 0x19\n";
unpack('L<', substr($index_file, 0, 4)) == 0x1a
    or die "indices do not start with chunk 0x1a\n";

my $vertex_size = unpack('L<', substr($vertex_file, 4, 4));
my $index_size = unpack('L<', substr($index_file, 4, 4));
length($vertex_file) == 8 + $vertex_size or die "invalid vertex chunk size\n";
length($index_file) == 8 + $index_size or die "invalid index chunk size\n";

my @vertices = map {
    [unpack('f<3', substr($vertex_file, 8 + $_ * 12, 12))]
} 0 .. ($vertex_size / 12 - 1);
my @indices = unpack('L<*', substr($index_file, 8));

my %levels;
for (my $i = 0; $i < @indices; $i += 3) {
    my ($a, $b, $c) = @vertices[@indices[$i .. $i + 2]];
    my @ab = map { $b->[$_] - $a->[$_] } 0 .. 2;
    my @ac = map { $c->[$_] - $a->[$_] } 0 .. 2;
    my @cross = (
        $ab[1] * $ac[2] - $ab[2] * $ac[1],
        $ab[2] * $ac[0] - $ab[0] * $ac[2],
        $ab[0] * $ac[1] - $ab[1] * $ac[0],
    );
    my $twice_area = sqrt($cross[0]**2 + $cross[1]**2 + $cross[2]**2);
    next if $twice_area == 0;
    next if abs($cross[1]) / $twice_area < 0.98;
    my $minimum_y = ($a->[1] < $b->[1] ? $a->[1] : $b->[1]);
    $minimum_y = $c->[1] if $c->[1] < $minimum_y;
    my $maximum_y = ($a->[1] > $b->[1] ? $a->[1] : $b->[1]);
    $maximum_y = $c->[1] if $c->[1] > $maximum_y;
    next if $maximum_y - $minimum_y > 0.05;

    my $level = sprintf('%.1f', ($a->[1] + $b->[1] + $c->[1]) / 3);
    my $entry = ($levels{$level} //= {
        area => 0,
        weighted_x => 0,
        weighted_z => 0,
        min_x => 1e30,
        max_x => -1e30,
        min_z => 1e30,
        max_z => -1e30,
        triangles => 0,
    });
    my $area = $twice_area / 2;
    my $centroid_x = ($a->[0] + $b->[0] + $c->[0]) / 3;
    my $centroid_z = ($a->[2] + $b->[2] + $c->[2]) / 3;
    $entry->{area} += $area;
    $entry->{weighted_x} += $centroid_x * $area;
    $entry->{weighted_z} += $centroid_z * $area;
    $entry->{triangles}++;
    for my $vertex ($a, $b, $c) {
        $entry->{min_x} = $vertex->[0] if $vertex->[0] < $entry->{min_x};
        $entry->{max_x} = $vertex->[0] if $vertex->[0] > $entry->{max_x};
        $entry->{min_z} = $vertex->[2] if $vertex->[2] < $entry->{min_z};
        $entry->{max_z} = $vertex->[2] if $vertex->[2] > $entry->{max_z};
    }
}

for my $level (sort { $b <=> $a } keys %levels) {
    my $entry = $levels{$level};
    next if $entry->{area} < 0.5;
    printf "y=%6.1f area=%8.2f triangles=%5d centroid=(%8.3f,%8.3f) bounds=(%8.3f..%8.3f,%8.3f..%8.3f)\n",
        $level,
        $entry->{area},
        $entry->{triangles},
        $entry->{weighted_x} / $entry->{area},
        $entry->{weighted_z} / $entry->{area},
        $entry->{min_x},
        $entry->{max_x},
        $entry->{min_z},
        $entry->{max_z};
}
