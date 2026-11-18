package dev.litospherity.fabric;

import net.fabricmc.api.ModInitializer;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/** Ф2: мод грузится, поднимает ядро и пишет в лог его версию. Генерации пока нет (Ф3). */
public final class LithoMod implements ModInitializer {
    public static final String MOD_ID = "litospherity";
    public static final Logger LOG = LoggerFactory.getLogger(MOD_ID);

    @Override
    public void onInitialize() {
        try {
            LithoNative core = LithoNative.get();
            LOG.info("Litospherity: ядро {} (ABI {}), полей {}, платформа {}",
                    core.coreVersion(), core.abiVersion(), core.fieldCount(), LithoNative.platform());
            try (LithoNative.Generator probe = core.newGenerator(42L, 1, 2)) {
                float[] f = probe.fields(new double[] {0.0, 0.0});
                LOG.info("Litospherity: проба сид 42 gameplay v1-dev (0,0): C={} E={} W={} T={} H={}",
                        f[0], f[1], f[2], f[3], f[4]);
            }
        } catch (Throwable error) {
            LOG.error("Litospherity: нативное ядро не загрузилось", error);
            throw new IllegalStateException("Litospherity: нет нативного ядра для " + LithoNative.platform(), error);
        }
    }
}
