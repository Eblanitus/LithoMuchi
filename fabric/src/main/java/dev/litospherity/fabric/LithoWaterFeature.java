package dev.litospherity.fabric;

import com.mojang.serialization.MapCodec;
import java.util.concurrent.atomic.AtomicLong;
import net.minecraft.core.BlockPos;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.material.Fluids;

/**
 * Ф6-В3/В4 (PLAN-WATER-FABRIC): вода выше уровня моря на Fabric.
 *
 * <p>Заливка по колонкам и герметизация одним проходом на шаге {@code RAW_GENERATION}
 * (запасной путь раздела 3 плана): синхронная точка заливки {@code NoiseBasedChunkGenerator}
 * в 26.3 без компилятора не проверяется, а фича — обычный API, который переживает C2ME.
 * Плата: дно под водой правится здесь же, а не шагом SURFACE.
 *
 * <ul>
 *   <li>уровень и дно колонки берутся у ядра ({@code litho_water_batch}), один вызов FFM на чанк
 *       (18×18 колонок: чанк плюс блок по краю для герметизации);</li>
 *   <li>вода ставится от {@code level − 1} вниз до дна, пока блок — воздух;</li>
 *   <li>если сосед по стороне — воздух, а у ядра воды на этой высоте нет, туда ставится блок
 *       берега (заделка): это метрика, а не норма (шаг В5);</li>
 *   <li>верхний блок под водой меняется на землю, как у подводного дна.</li>
 * </ul>
 *
 * <p>ВО2 (PLAN-WATER-UPDATE §8.2): работает в мире с пресетом Litospherity
 * ({@code v1-dev-w}); в остальных мирах возвращает {@code false}. Материалы,
 * поток и вид воды берутся из битов {@code info} ({@code litho_water_batch2}).
 */
public final class LithoWaterFeature implements Feature {
    /** Уровень моря: выше него — предмет R2. */
    public static final int SEA_LEVEL = 63;
    /** Колонка чанка плюс один блок по краю: герметизации нужны соседи. */
    private static final int SIDE = 18;
    /** Сколько блоков над зеркалом можно убрать (§8.1 `clear_above_blocks`). */
    private static final int CLEAR_ABOVE = 6;
    private static final int[][] SIDES = {{1, 0}, {-1, 0}, {0, 1}, {0, -1}};

    /** Заделок герметизации (метрика В5: ≤ 2 на чанк с водой выше моря). */
    public static final AtomicLong SEALS = new AtomicLong();
    /** Чанков, где заливалась вода выше моря. */
    public static final AtomicLong CHUNKS = new AtomicLong();
    /** Колонок, где вода выше моря поставлена. */
    public static final AtomicLong COLUMNS = new AtomicLong();
    private static final byte KIND_RIVER = 1;
    public static volatile boolean LIVE_STEPS = Boolean.getBoolean("litospherity.water.liveSteps");
    public static final AtomicLong STEPS = new AtomicLong();

    /** 26.3: {@code Feature} — интерфейс без параметра конфигурации, а реестр
     *  {@code FEATURE_TYPE} держит кодеки. Типу воды конфигурация не нужна. */
    public static final MapCodec<LithoWaterFeature> CODEC = MapCodec.unit(new LithoWaterFeature());

    @Override
    public MapCodec<LithoWaterFeature> codec() {
        return CODEC;
    }

    @Override
    public boolean place(WorldGenLevel level, ChunkGenerator chunkGenerator, RandomSource random, BlockPos origin) {
        LithoFields.Binding binding = LithoFields.world();
        if (binding == null || binding.version() != LithoFields.VERSION_W) return false;
        int x0 = origin.getX() & ~15;
        int z0 = origin.getZ() & ~15;
        // Один вызов ядра на чанк: 18×18 колонок, начиная с блока левее/выше чанка.
        double[] xs = new double[SIDE * SIDE];
        double[] zs = new double[SIDE * SIDE];
        for (int j = 0; j < SIDE; j++) {
            for (int i = 0; i < SIDE; i++) {
                xs[j * SIDE + i] = x0 - 1 + i;
                zs[j * SIDE + i] = z0 - 1 + j;
            }
        }
        // Генератор общий и потокобезопасный (Ф1): не закрываем его здесь.
        LithoNative.Generator core = LithoFields.generator(binding);
        LithoNative.WaterColumns water = core.water(xs, zs);
        int minY = level.getMinY();
        int maxY = level.getMaxY();
        int placed = 0;
        long seals = 0;
        long steps = 0;
        BlockState waterState = Blocks.WATER.defaultBlockState();
        // Record the bottommost fluid actually placed. The kernel bed minus
        // two is only a search limit for an open cave, not a wet face.
        int[] lowestSource = new int[SIDE * SIDE];
        java.util.Arrays.fill(lowestSource, Integer.MAX_VALUE);
        // Заливка и правка дна.
        for (int j = 1; j <= 16; j++) {
            for (int i = 1; i <= 16; i++) {
                int k = j * SIDE + i;
                int surface = water.level()[k];
                if (surface == LithoNative.WaterColumns.NONE || surface < SEA_LEVEL) continue;
                // Уровень 63 — русло у моря (вода до y = 62): ядро отдаёт его, чтобы
                // прорезать русло по настоящим блокам. 3D-шум поднимает дно выше 62,
                // и без этого ванильная вода стояла в русле лужами.
                if (surface == SEA_LEVEL && water.kind()[k] != KIND_RIVER) continue;
                int x = (int) xs[k];
                int z = (int) zs[k];
                int from = Math.min(surface - 1, maxY);
                // bed is the highest solid block, not the lowest source. Keep
                // at least one source when the fractional bank rounds to level-1.
                int floor = Math.min(water.bed()[k], from - 1);
                // ВО2-1: над водой твёрдое снимаем на CLEAR_ABOVE блоков. Если
                // твёрдое продолжается дальше — колонка идёт в метрику
                // «накрытая вода» (§8.3), а не режется бесконечно.
                for (int y = from + 1; y <= Math.min(from + CLEAR_ABOVE, maxY); y++) {
                    BlockPos pos = new BlockPos(x, y, z);
                    BlockState above = level.getBlockState(pos);
                    // The receiver recreates permitted incoming steps/falls below.
                    // Do not preserve unrelated aquifer sources above this mirror.
                    if (above.isAir() || above.is(Blocks.BEDROCK)) continue;
                    level.setBlock(pos, Blocks.AIR.defaultBlockState(), 2);
                }
                int lowest = from + 1;
                for (int y = from; y >= Math.max(minY, floor - 2); y--) {
                    BlockPos pos = new BlockPos(x, y, z);
                    BlockState old = level.getBlockState(pos);
                    if (old.is(Blocks.BEDROCK) || (y <= floor && !old.isAir())) break;
                    if (!old.equals(waterState)) level.setBlock(pos, waterState, 2);
                    lowest = y;
                }
                if (lowest > from) continue;
                lowestSource[k] = lowest;
                placed++;
                COLUMNS.incrementAndGet();
                if (lowest <= minY) continue;
                BlockPos under = new BlockPos(x, lowest - 1, z);
                BlockState old = level.getBlockState(under);
                if (old.isAir()) {
                    level.setBlock(under, Blocks.STONE.defaultBlockState(), 2);
                    seals++;
                } else if (!old.is(Blocks.BEDROCK)) {
                    BlockState bedMat = material(LithoNative.WaterColumns.bedMat(water.info()[k]));
                    if (bedMat != null) level.setBlock(under, bedMat, 2);
                }
            }
        }
        // Герметизация: вода не должна уходить в воздух за берегом.
        for (int j = 1; j <= 16; j++) {
            for (int i = 1; i <= 16; i++) {
                int k = j * SIDE + i;
                int surface = water.level()[k];
                if (surface == LithoNative.WaterColumns.NONE || surface <= SEA_LEVEL || lowestSource[k] == Integer.MAX_VALUE) continue;
                // Only dry neighbours receive solid seals. In particular a
                // level-63 river or a lower lake is not a hole to fill with dirt.
                int top = Math.min(surface - 1, maxY);
                if (incomingTop(water, k) >= surface) top = Math.min(surface, maxY);
                for (int[] d : SIDES) {
                    int nk = k + d[1] * SIDE + d[0];
                    if (water.level()[nk] != LithoNative.WaterColumns.NONE) continue;
                    for (int y = top; y >= lowestSource[k]; y--) {
                        BlockPos pos = new BlockPos((int) xs[nk], y, (int) zs[nk]);
                        if (level.getBlockState(pos).isAir()) {
                            level.setBlock(pos, Blocks.DIRT.defaultBlockState(), 2);
                            seals++;
                        }
                    }
                }
            }
        }
        // ВО2-2 (§8.2 п. 2): пляж. Суша в 3 блоках от воды получает материал
        // берега, если её верхний блок не выше уровня воды на 2 блока.
        for (int j = 1; j <= 16; j++) {
            for (int i = 1; i <= 16; i++) {
                int k = j * SIDE + i;
                if (water.level()[k] != LithoNative.WaterColumns.NONE) continue;
                int shoreCode = LithoNative.WaterColumns.shoreMat(water.info()[k]);
                BlockState shore = material(shoreCode);
                if (shore == null) continue;
                int top = level.getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.OCEAN_FLOOR_WG,
                        (int) xs[k], (int) zs[k]) - 1;
                int waterLevel = water.info()[k] >>> 19;
                if (top < minY || top > waterLevel + 2) continue;
                BlockPos pos = new BlockPos((int) xs[k], top, (int) zs[k]);
                if (isSurface(level.getBlockState(pos))) {
                    level.setBlock(pos, shore, 2);
                }
            }
        }
        // Write the lip in the RECEIVING chunk. The one-block prediction halo
        // is available even when the upper chunk has not run its feature yet.
        // This pass is after clearing: chunk order cannot erase an intended
        // fall or leave a stale extra source above the mirror.
        for (int j = 1; j <= 16; j++) {
            for (int i = 1; i <= 16; i++) {
                int k = j * SIDE + i;
                if (lowestSource[k] == Integer.MAX_VALUE) continue;
                int lower = water.level()[k];
                int top = incomingTop(water, k);
                if (top < lower) continue;
                boolean fall = top > lower;
                BlockState flow = fall ? fallingWater() : Fluids.FLOWING_WATER.getFlowing(7, false).createLegacyBlock();
                for (int y = Math.max(lower, minY); y <= Math.min(top, maxY); y++) {
                    BlockPos pos = new BlockPos((int) xs[k], y, (int) zs[k]);
                    BlockState old = level.getBlockState(pos);
                    if (old.is(Blocks.BEDROCK) || (!fall && !old.isAir())) continue;
                    level.setBlock(pos, flow, 2);
                    if (!fall) {
                        if (LIVE_STEPS) level.scheduleTick(pos, Fluids.FLOWING_WATER, 1);
                        steps++;
                    }
                }
            }
        }
        if (placed > 0) CHUNKS.incrementAndGet();
        SEALS.addAndGet(seals);
        STEPS.addAndGet(steps);
        return placed > 0;
    }

    /** Highest incoming fluid block, or NONE. One-block steps must have a
     * predicted upper source at this face; only a marked fall spans a tall drop. */
    private static int incomingTop(LithoNative.WaterColumns water, int k) {
        int lower = water.level()[k];
        int top = LithoNative.WaterColumns.NONE;
        for (int[] d : SIDES) {
            int nk = k + d[1] * SIDE + d[0];
            int upper = water.level()[nk];
            if (water.kind()[nk] != KIND_RIVER || upper <= lower) continue;
            if (upper - lower >= 2 && LithoNative.WaterColumns.flow(water.info()[nk]) == 4) {
                top = Math.max(top, upper - 1);
            } else if (water.kind()[k] == KIND_RIVER && upper == lower + 1) {
                top = Math.max(top, lower);
            }
        }
        return top;
    }

    /** ВО2-2 (§7.5): блоки материалов по коду из битов info. */
    private static BlockState material(int code) {
        return switch (code) {
            case 1 -> Blocks.SAND.defaultBlockState();
            case 2 -> Blocks.GRAVEL.defaultBlockState();
            case 3 -> Blocks.CLAY.defaultBlockState();
            case 4 -> Blocks.COBBLESTONE.defaultBlockState();
            case 5 -> Blocks.MUD.defaultBlockState();
            case 6 -> Blocks.MOSS_BLOCK.defaultBlockState();
            case 7 -> Blocks.TUFF.defaultBlockState();
            case 8 -> Blocks.CALCITE.defaultBlockState();
            case 9 -> Blocks.COARSE_DIRT.defaultBlockState();
            default -> null;
        };
    }

    /** ВО2-2 (§8.2 п. 3): падающая вода между двумя уровнями русла. */
    private static BlockState fallingWater() {
        return Fluids.FLOWING_WATER.getFlowing(7, true).createLegacyBlock();
    }

    private static boolean isSurface(BlockState state) {
        return state.is(Blocks.GRASS_BLOCK)
                || state.is(Blocks.PODZOL)
                || state.is(Blocks.MYCELIUM)
                || state.is(Blocks.COARSE_DIRT)
                || state.is(Blocks.ROOTED_DIRT)
                || state.is(Blocks.MOSS_BLOCK)
                || state.is(Blocks.SNOW_BLOCK)
                || state.is(Blocks.DIRT_PATH)
                || state.is(Blocks.FARMLAND)
                || state.is(Blocks.MUD)
                || state.is(Blocks.SAND);
    }
}
