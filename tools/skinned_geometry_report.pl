#!/usr/bin/env perl
# SPDX-License-Identifier: GPL-3.0-or-later
use strict;
use warnings;

sub read_file {
    my ($path) = @_;
    open my $fh, '<:raw', $path or die "cannot open $path: $!\n";
    local $/;
    return <$fh>;
}

sub u16 {
    my ($bytes, $offset) = @_;
    die "truncated u16 at $offset\n" if $offset + 2 > length $bytes;
    return unpack 'v', substr($bytes, $offset, 2);
}

sub u32 {
    my ($bytes, $offset) = @_;
    die "truncated u32 at $offset\n" if $offset + 4 > length $bytes;
    return unpack 'V', substr($bytes, $offset, 4);
}

sub f32 {
    my ($bytes, $offset) = @_;
    die "truncated f32 at $offset\n" if $offset + 4 > length $bytes;
    return unpack 'f<', substr($bytes, $offset, 4);
}

sub bind_pose_names {
    my ($path) = @_;
    my $bytes = read_file($path);
    my $count = u16($bytes, 0);
    my $offset = 2;
    my @names;
    for my $index (0 .. $count - 1) {
        my $end = index($bytes, "\0", $offset);
        die "unterminated bind-pose bone $index\n" if $end < 0;
        push @names, substr($bytes, $offset, $end - $offset);
        $offset = $end + 1 + 36;
    }
    die "trailing or truncated bind-pose data\n" if $offset != length $bytes;
    return @names;
}

sub parse_model {
    my ($path) = @_;
    my $bytes = read_file($path);
    my ($vertices, $indices);
    my $offset = 0;
    while ($offset < length $bytes) {
        my $id = u32($bytes, $offset);
        my $size = u32($bytes, $offset + 4);
        my $payload = substr($bytes, $offset + 8, $size);
        die "truncated chunk $id\n" if length($payload) != $size;
        $vertices = $payload if $id == 3;
        $indices = $payload if $id == 4;
        $offset += 8 + $size;
    }
    die "missing vertex or index chunk in $path\n"
        unless defined $vertices && defined $indices;
    return ($vertices, $indices);
}

sub report {
    my ($label, $bind_path, $model_path) = @_;
    my @bones = bind_pose_names($bind_path);
    my ($vertices, $indices) = parse_model($model_path);
    my $vertex_count = u32($vertices, 0);
    my $index_count = u32($indices, 0);
    die "bad vertex chunk size\n"
        if length($vertices) != 4 + 48 * $vertex_count;
    die "bad index chunk size\n"
        if length($indices) != 4 + 2 * $index_count;

    my %used;
    for my $index (0 .. $index_count - 1) {
        my $vertex_index = u16($indices, 4 + 2 * $index);
        die "index $vertex_index exceeds $vertex_count\n"
            if $vertex_index >= $vertex_count;
        $used{$vertex_index} = 1;
    }

    my (@min, @max);
    my %stats;
    for my $vertex_index (sort { $a <=> $b } keys %used) {
        my $base = 4 + 48 * $vertex_index;
        my @position = map { f32($vertices, $base + 4 * $_) } 0 .. 2;
        for my $axis (0 .. 2) {
            $min[$axis] = $position[$axis]
                if !defined($min[$axis]) || $position[$axis] < $min[$axis];
            $max[$axis] = $position[$axis]
                if !defined($max[$axis]) || $position[$axis] > $max[$axis];
        }
        my @indices = unpack 'C4', substr($vertices, $base + 12, 4);
        my @weights = map { f32($vertices, $base + 16 + 4 * $_) } 0 .. 2;
        push @weights, 1.0 - $weights[0] - $weights[1] - $weights[2];
        for my $slot (0 .. 3) {
            next if $weights[$slot] <= 0.0001;
            my $bone = $indices[$slot];
            die "bone index $bone exceeds bind pose\n" if $bone >= @bones;
            my $stat = ($stats{$bones[$bone]} //= {
                weight => 0.0,
                weighted_position => [0.0, 0.0, 0.0],
                min => [],
                max => [],
            });
            $stat->{weight} += $weights[$slot];
            for my $axis (0 .. 2) {
                $stat->{weighted_position}[$axis] +=
                    $position[$axis] * $weights[$slot];
                $stat->{min}[$axis] = $position[$axis]
                    if !defined($stat->{min}[$axis])
                    || $position[$axis] < $stat->{min}[$axis];
                $stat->{max}[$axis] = $position[$axis]
                    if !defined($stat->{max}[$axis])
                    || $position[$axis] > $stat->{max}[$axis];
            }
        }
    }

    print "model=$label vertices=$vertex_count used_vertices=" . scalar(keys %used)
        . " indices=$index_count\n";
    printf "bounds x=[%.5f,%.5f] y=[%.5f,%.5f] z=[%.5f,%.5f]\n",
        $min[0], $max[0], $min[1], $max[1], $min[2], $max[2];
    for my $bone (@bones) {
        next unless $bone =~ /^(?:Chest|Spin|Left|Right)/;
        my $stat = $stats{$bone} or next;
        my @centroid = map {
            $stat->{weighted_position}[$_] / $stat->{weight}
        } 0 .. 2;
        printf "%-20s weight=%9.3f centroid=(%8.5f,%8.5f,%8.5f)"
            . " bounds=x[%8.5f,%8.5f] y[%8.5f,%8.5f] z[%8.5f,%8.5f]\n",
            $bone, $stat->{weight}, @centroid,
            $stat->{min}[0], $stat->{max}[0],
            $stat->{min}[1], $stat->{max}[1],
            $stat->{min}[2], $stat->{max}[2];
    }
}

die "usage: $0 LABEL BIND_POSE CONVERTED_MODEL [LABEL BIND_POSE CONVERTED_MODEL ...]\n"
    if !@ARGV || @ARGV % 3;
while (@ARGV) {
    report(splice @ARGV, 0, 3);
    print "\n" if @ARGV;
}
