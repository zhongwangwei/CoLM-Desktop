//! 最小 ESRI Polygon SHP 读取器：只为流域格心 mask 服务。

use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub west: f64,
    pub east: f64,
    pub south: f64,
    pub north: f64,
}

#[derive(Debug, Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone)]
pub struct PolygonDomain {
    records: Vec<Vec<Vec<Point>>>,
    bounds: Bounds,
}

impl PolygonDomain {
    pub fn read(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        validate_wgs84_prj(path)?;
        let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        if bytes.len() < 100 || be_u32(&bytes, 0)? != 9994 || le_u32(&bytes, 28)? != 1000 {
            bail!("{} is not an ESRI shapefile", path.display());
        }
        let header_type = le_u32(&bytes, 32)?;
        if !matches!(header_type, 5 | 15 | 25) {
            bail!("shapefile must contain Polygon, PolygonZ, or PolygonM records");
        }
        let declared_len = usize::try_from(be_u32(&bytes, 24)?)?
            .checked_mul(2)
            .context("shapefile length overflows usize")?;
        if declared_len < 100 || declared_len > bytes.len() {
            bail!("shapefile header declares a truncated file");
        }

        let mut offset = 100;
        let mut records = Vec::new();
        let mut bounds = Bounds {
            west: f64::INFINITY,
            east: f64::NEG_INFINITY,
            south: f64::INFINITY,
            north: f64::NEG_INFINITY,
        };
        while offset < declared_len {
            let header_end = offset.checked_add(8).context("record header overflow")?;
            if header_end > declared_len {
                bail!("truncated shapefile record header");
            }
            let content_len = usize::try_from(be_u32(&bytes, offset + 4)?)?
                .checked_mul(2)
                .context("record length overflows usize")?;
            let end = header_end
                .checked_add(content_len)
                .context("record end overflows usize")?;
            if end > declared_len {
                bail!("truncated shapefile record");
            }
            if let Some(rings) = read_polygon_record(&bytes[header_end..end], &mut bounds)? {
                records.push(rings);
            }
            offset = end;
        }
        if records.is_empty() {
            bail!("shapefile contains no polygon records");
        }
        if bounds.east - bounds.west > 180.0 {
            bail!("dateline-crossing shapefiles are not supported yet");
        }
        Ok(Self { records, bounds })
    }

    pub fn bounds(&self) -> Bounds {
        self.bounds
    }

    /// ESRI 同一 record 内多 ring 用奇偶规则表达 shell/hole；
    /// 多 record 取 union。点在任一 ring 边界上时视为域内。
    pub fn contains(&self, lon: f64, lat: f64) -> bool {
        let point = Point { x: lon, y: lat };
        self.records.iter().any(|rings| {
            let mut inside = false;
            for ring in rings {
                match relation(point, ring) {
                    Relation::Boundary => return true,
                    Relation::Inside => inside = !inside,
                    Relation::Outside => {}
                }
            }
            inside
        })
    }
}

fn validate_wgs84_prj(path: &Path) -> Result<()> {
    let prj = [path.with_extension("prj"), path.with_extension("PRJ")]
        .into_iter()
        .find(|candidate| candidate.is_file())
        .with_context(|| format!("{} needs a WGS84 .prj file", path.display()))?;
    let text = fs::read_to_string(&prj)
        .with_context(|| format!("cannot read shapefile CRS {}", prj.display()))?
        .to_ascii_uppercase();
    let is_wgs84 = text.contains("WGS_1984")
        || text.contains("WGS 84")
        || text.contains("EPSG\",4326")
        || text.contains("EPSG:4326");
    let projected = text.contains("PROJCS[") || text.contains("PROJCRS[");
    if !is_wgs84 || projected {
        bail!(
            "{} is not an unprojected WGS84 shapefile; reproject it to EPSG:4326 first",
            path.display()
        );
    }
    Ok(())
}

fn read_polygon_record(content: &[u8], bounds: &mut Bounds) -> Result<Option<Vec<Vec<Point>>>> {
    if content.len() < 4 {
        bail!("truncated shapefile record");
    }
    let shape_type = le_u32(content, 0)?;
    if shape_type == 0 {
        return Ok(None);
    }
    if !matches!(shape_type, 5 | 15 | 25) || content.len() < 44 {
        bail!("shapefile record is not a valid polygon");
    }
    let part_count = usize::try_from(le_u32(content, 36)?)?;
    let point_count = usize::try_from(le_u32(content, 40)?)?;
    if part_count == 0 || point_count == 0 || part_count > point_count {
        bail!("polygon has invalid part/point counts");
    }
    let points_start = 44usize
        .checked_add(part_count.checked_mul(4).context("parts size overflow")?)
        .context("points offset overflow")?;
    let points_end = points_start
        .checked_add(
            point_count
                .checked_mul(16)
                .context("points size overflow")?,
        )
        .context("points end overflow")?;
    if points_end > content.len() {
        bail!("polygon record is truncated");
    }

    let mut starts = (0..part_count)
        .map(|index| usize::try_from(le_u32(content, 44 + index * 4)?).map_err(Into::into))
        .collect::<Result<Vec<_>>>()?;
    if starts.first() != Some(&0) {
        bail!("polygon first part must start at point zero");
    }
    starts.push(point_count);

    let mut rings = Vec::with_capacity(part_count);
    for span in starts.windows(2) {
        if span[0] >= span[1] || span[1] > point_count {
            bail!("polygon has an invalid part index");
        }
        let mut ring = (span[0]..span[1])
            .map(|index| {
                let offset = points_start + index * 16;
                Ok(Point {
                    x: le_f64(content, offset)?,
                    y: le_f64(content, offset + 8)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if ring
            .first()
            .zip(ring.last())
            .is_none_or(|(a, b)| !same(*a, *b))
        {
            bail!("polygon ring is not closed");
        }
        ring.pop();
        ring.dedup_by(|a, b| same(*a, *b));
        if ring.len() < 3 || signed_area(&ring).abs() <= f64::EPSILON {
            bail!("polygon ring is degenerate");
        }
        for point in &ring {
            if !point.x.is_finite()
                || !point.y.is_finite()
                || !(-180.0..=180.0).contains(&point.x)
                || !(-90.0..=90.0).contains(&point.y)
            {
                bail!("shapefile coordinate is outside WGS84 longitude/latitude bounds");
            }
            bounds.west = bounds.west.min(point.x);
            bounds.east = bounds.east.max(point.x);
            bounds.south = bounds.south.min(point.y);
            bounds.north = bounds.north.max(point.y);
        }
        rings.push(ring);
    }
    Ok(Some(rings))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Relation {
    Outside,
    Inside,
    Boundary,
}

fn relation(point: Point, ring: &[Point]) -> Relation {
    let mut inside = false;
    for index in 0..ring.len() {
        let a = ring[index];
        let b = ring[(index + 1) % ring.len()];
        if on_segment(point, a, b) {
            return Relation::Boundary;
        }
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
    }
    if inside {
        Relation::Inside
    } else {
        Relation::Outside
    }
}

fn on_segment(point: Point, a: Point, b: Point) -> bool {
    let cross = (point.x - a.x) * (b.y - a.y) - (point.y - a.y) * (b.x - a.x);
    let scale = (b.x - a.x).abs().max((b.y - a.y).abs()).max(1.0);
    cross.abs() <= 1.0e-12 * scale
        && point.x >= a.x.min(b.x) - 1.0e-12
        && point.x <= a.x.max(b.x) + 1.0e-12
        && point.y >= a.y.min(b.y) - 1.0e-12
        && point.y <= a.y.max(b.y) + 1.0e-12
}

fn signed_area(ring: &[Point]) -> f64 {
    (0..ring.len())
        .map(|index| {
            let a = ring[index];
            let b = ring[(index + 1) % ring.len()];
            a.x * b.y - b.x * a.y
        })
        .sum::<f64>()
        * 0.5
}

fn same(a: Point, b: Point) -> bool {
    a.x == b.x && a.y == b.y
}

fn be_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value: [u8; 4] = bytes
        .get(offset..offset + 4)
        .context("truncated big-endian integer")?
        .try_into()?;
    Ok(u32::from_be_bytes(value))
}

fn le_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value: [u8; 4] = bytes
        .get(offset..offset + 4)
        .context("truncated little-endian integer")?
        .try_into()?;
    Ok(u32::from_le_bytes(value))
}

fn le_f64(bytes: &[u8], offset: usize) -> Result<f64> {
    let value: [u8; 8] = bytes
        .get(offset..offset + 8)
        .context("truncated little-endian float")?
        .try_into()?;
    Ok(f64::from_le_bytes(value))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn fixture() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("colm-watershed-{}-{nonce}.shp", std::process::id()));
        let outer = [
            (-2.0, -2.0),
            (-2.0, 2.0),
            (2.0, 2.0),
            (2.0, -2.0),
            (-2.0, -2.0),
        ];
        let hole = [
            (-1.0, -1.0),
            (1.0, -1.0),
            (1.0, 1.0),
            (-1.0, 1.0),
            (-1.0, -1.0),
        ];
        let points = outer.into_iter().chain(hole).collect::<Vec<_>>();
        let mut content = vec![0; 44 + 2 * 4 + points.len() * 16];
        put_le_u32(&mut content, 0, 5);
        put_box(&mut content, 4, [-2.0, -2.0, 2.0, 2.0]);
        put_le_u32(&mut content, 36, 2);
        put_le_u32(&mut content, 40, u32::try_from(points.len()).unwrap());
        put_le_u32(&mut content, 44, 0);
        put_le_u32(&mut content, 48, u32::try_from(outer.len()).unwrap());
        for (index, (x, y)) in points.into_iter().enumerate() {
            put_le_f64(&mut content, 52 + index * 16, x);
            put_le_f64(&mut content, 60 + index * 16, y);
        }

        let file_len = 100 + 8 + content.len();
        let mut bytes = vec![0; file_len];
        put_be_u32(&mut bytes, 0, 9994);
        put_be_u32(&mut bytes, 24, u32::try_from(file_len / 2).unwrap());
        put_le_u32(&mut bytes, 28, 1000);
        put_le_u32(&mut bytes, 32, 5);
        put_box(&mut bytes, 36, [-2.0, -2.0, 2.0, 2.0]);
        put_be_u32(&mut bytes, 100, 1);
        put_be_u32(&mut bytes, 104, u32::try_from(content.len() / 2).unwrap());
        bytes[108..].copy_from_slice(&content);
        fs::write(&path, bytes).unwrap();
        fs::write(
            path.with_extension("prj"),
            r#"GEOGCS["WGS 84",DATUM["WGS_1984"]]"#,
        )
        .unwrap();
        path
    }

    #[test]
    fn polygon_shapefile_keeps_shell_hole_and_boundary_semantics() {
        let domain = PolygonDomain::read(fixture()).unwrap();
        assert_eq!(
            domain.bounds(),
            Bounds {
                west: -2.0,
                east: 2.0,
                south: -2.0,
                north: 2.0
            }
        );
        assert!(domain.contains(1.5, 0.0));
        assert!(!domain.contains(0.0, 0.0));
        assert!(domain.contains(-2.0, 0.0));
        assert!(!domain.contains(3.0, 0.0));

        let grid = crate::Grid {
            nlon: 360,
            nlat: 180,
        };
        let mesh = crate::mesh::EqualLatLonMesh::from_polygon(grid, &domain).unwrap();
        assert_eq!(mesh.window.nlon, 4);
        assert_eq!(mesh.window.nlat, 4);
        assert_eq!(mesh.summary().unwrap().active_cells, 12);
    }

    fn put_be_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
    }

    fn put_le_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put_le_f64(bytes: &mut [u8], offset: usize, value: f64) {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn put_box(bytes: &mut [u8], offset: usize, values: [f64; 4]) {
        for (index, value) in values.into_iter().enumerate() {
            put_le_f64(bytes, offset + index * 8, value);
        }
    }
}
