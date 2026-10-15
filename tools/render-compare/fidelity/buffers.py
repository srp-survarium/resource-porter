"""numpy decoders for captured render-target dumps (1280x720 unless given)."""
import numpy as np


def _f11(v, bits):
    e = (v >> bits) & 31; m = v & ((1 << bits)-1)
    return np.where(e == 0, m/(1 << bits)*2.0**-14, (1+m/(1 << bits))*2.0**(e.astype(float)-15))


def r26(path, w=1280, h=720):
    """R11G11B10_FLOAT."""
    x = np.fromfile(path, np.uint32)[:w*h].reshape(h, w)
    return np.stack([_f11(x & 2047, 6), _f11((x >> 11) & 2047, 6), _f11(x >> 22, 5)], -1)


def r28(path, w=1280, h=720):
    """R8G8B8A8_UNORM, all four channels."""
    return np.fromfile(path, np.uint8)[:w*h*4].reshape(h, w, 4)/255.


def r24(path, w=1280, h=720):
    """R10G10B10A2_UNORM, RGB only (0.11e sqrt-encoded light accumulators)."""
    x = np.fromfile(path, np.uint32)[:w*h].reshape(h, w)
    return np.stack([(x >> (10*c)) & 1023 for c in range(3)], -1)/1023.
