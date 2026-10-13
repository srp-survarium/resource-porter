#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Make six separated eight-heading panorama stations and four detailed views."""
import argparse
import json
import math
from pathlib import Path

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('output',type=Path)
a=parser.parse_args()
# Authored respawn / battery-spawner coordinates, raised by 1.6m for eye height.
stations=[
 ('east_hall',[102.81500244140625,8.17200002670288,82.78099822998047]),
 ('west_factory',[-83.90499877929688,13.766999816894531,25.89900016784668]),
 ('south_yard',[29.993999481201172,4.011999940872192,-.828000009059906]),
 ('north_yard',[-2.311000108718872,3.246999955177307,114.41200256347656]),
 ('central_upper',[19.76799964904785,8.992000198364257,57.034000396728516]),
 ('northeast_yard',[41.720001220703125,3.303999996185303,127.5530014038086]),
]
poses=[]
for station,position in stations:
 for heading in range(0,360,45):
  angle=math.radians(heading)
  poses.append({'name':f'{station}_{heading:03d}','station':station,'heading_degrees':heading,
   'position':position,'forward':[math.sin(angle),0,math.cos(angle)],'fov':65,'aspect':1280/720,
   'near':.05,'far':500,'settle_ms':2000,'split':'diagnostic' if station in ('east_hall','south_yard','central_upper') else 'heldout'})
for name,position,target in [
 ('lamp_east',[102.815,8.172,82.781],[106.172,10.909,81.908]),
 ('lamp_west',[-80.0,13.767,32.0],[-82.747,16.852,33.073]),
 ('central_shadow',[7.806,11.717,31.919],[13.779,7.235,55.766]),
 ('panoramic_overlook',[20,30,65],[20,2,110]),
]:
 direction=[b-c for b,c in zip(target,position)];length=math.sqrt(sum(x*x for x in direction))
 poses.append({'name':name,'position':position,'forward':[x/length for x in direction],'fov':65,'settle_ms':2000,'split':'diagnostic'})
with a.output.open('x') as f:json.dump(poses,f,indent=2);f.write('\n')
print(f'{len(poses)} views, {len(stations)} panorama stations, 24 held-out headings')
