"""Exercise the generated 26.3 density expression without redistributing Mojang data."""
import unittest
from fabric_resources import calm_surface_density


def sample(expr, calm, surface, y, vanilla):
    def at(node):
        if node == 'original':
            return vanilla
        if not isinstance(node, dict):
            return node
        op = node['type'].split(':')[1]
        if op == 'field':
            return {'calm': calm, 'offset': (surface - 128) / 128}[node['field']]
        if op == 'gradient':
            t = min(1, max(0, (y-node['from_coordinate'])/(node['to_coordinate']-node['from_coordinate'])))
            return (1-t)*node['from_value'] + t*node['to_value']
        if op == 'range_choice':
            return at(node['when_in_range' if node['min_inclusive'] <= at(node['input']) < node['max_exclusive'] else 'when_out_of_range'])
        if op == 'cache': return at(node['input'])
        if op == 'clamp': return min(node['max'], max(node['min'], at(node['input'])))
        if op == 'add': return at(node['left']) + at(node['right'])
        if op == 'mul': return at(node['left']) * at(node['right'])
        if op == 'lerp': return (1-at(node['alpha']))*at(node['first']) + at(node['alpha'])*at(node['second'])
        raise AssertionError(op)
    return at(expr)


class CalmSurfaceTest(unittest.TestCase):
    def test_corridor_bed_roof_and_deep_cave(self):
        expr = calm_surface_density('original')
        for surface in [62.75, 126.125, 348.125]:
            bed = round(surface)
            self.assertGreater(sample(expr, 0., surface, bed, -0.5), 0.)
            self.assertLess(sample(expr, 0., surface, bed+1, 0.5), 0.)
            self.assertLess(sample(expr, 0., surface, bed+24, 0.5), 0.)
            self.assertEqual(sample(expr, 0., surface, bed-4, -0.625), -0.625)
            self.assertEqual(sample(expr, 1., surface, bed, -0.625), -0.625)
            a = sample(expr, 0., surface, bed, -0.625)
            self.assertAlmostEqual(sample(expr, .5, surface, bed, -0.625), (a-.625)/2)

if __name__ == '__main__':
    unittest.main()
