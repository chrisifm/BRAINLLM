# Geo fields and filters

A field of kind `geo` stores a point as `{"lat": 19.4, "lng": -99.1}`, in degrees. Other shapes are 422 and a database check refuses them too.

```js
// schema.jsbrain
<meta server>
export default { name: 'str!', at: 'geo' }
// query.jsbrain
<meta server>
export default { filter: { at: ['near', 'bbox', 'exists'] } }
```

| Filter | Meaning |
|---|---|
| `?at[near]=lat,lng,km` | Great-circle distance up to `km` (above 0, at most 20040) |
| `?at[bbox]=minLat,minLng,maxLat,maxLng` | Inside the box; `minLng` above `maxLng` crosses the antimeridian |
| `?at[exists]=false` | Rows without a point |

Radius searches handle the antimeridian and the poles. Declaring a filter builds a GiST index on Postgres's own `point`
type: no database extension is involved, so there is nothing to install and no missing-extension failure.

Sorting by distance and returning the distance are not available. Distance assumes a spherical Earth (mean radius 6371.0088 km), which is off by up to about 0.5%.
