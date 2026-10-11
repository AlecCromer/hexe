from _common import *
lattice_case("sop-lattice-group", "lattice 3x2x4 cells, bezier, group '0 2 5-8' limits the deformed points", (3, 2, 4), (-1.5, -2, 0), (1.5, 2, 4), 1, [(0.1,0.3,1.1),(0.9,-1.5,2.8),(-0.7,1.0,0.2),(2.5,0.0,1.5),(0.0,-3.0,1.0),(0.0,0.0,-1.0),(-1.5,-2.0,0.0),(1.5,2.0,4.0),(0.35,0.45,1.25),(-0.5,-1.0,3.5)], group="0 2 5-8", seed=11)
