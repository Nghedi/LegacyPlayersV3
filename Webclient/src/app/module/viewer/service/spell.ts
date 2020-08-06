import {Injectable, OnDestroy} from "@angular/core";
import {Observable, of, Subscription} from "rxjs";
import {Localized} from "../../../domain_value/localized";
import {BasicSpell} from "../../../domain_value/data/basic_spell";
import {concatMap} from "rxjs/operators";
import {InstanceViewerMeta} from "../domain_value/instance_viewer_meta";
import {InstanceDataService} from "./instance_data";
import {DataService} from "../../../service/data";

@Injectable({
    providedIn: "root",
})
export class SpellService implements OnDestroy {

    private subscription: Subscription;
    private current_meta: InstanceViewerMeta;

    constructor(
        private instanceDataService: InstanceDataService,
        private dataService: DataService
    ) {
        this.subscription = this.instanceDataService.meta.subscribe(meta => this.current_meta = meta);
    }

    ngOnDestroy(): void {
        this.subscription?.unsubscribe();
    }

    get_localized_basic_spell(spell_id: number): Observable<Localized<BasicSpell>> {
        if (!this.current_meta)
            return of(this.dataService.unknown_basic_spell);
        if (spell_id === 0)
            return of({
                localization: "Auto Attack",
                base: {
                    id: 0,
                    icon: "inv_sword_04",
                    school: 0
                }
            });
        return this.dataService.get_server_by_id(this.current_meta.server_id)
            .pipe(concatMap(server => !server ? of(this.dataService.unknown_basic_spell)
                : this.dataService.get_localized_basic_spell(server.expansion_id, spell_id)));
    }
}
